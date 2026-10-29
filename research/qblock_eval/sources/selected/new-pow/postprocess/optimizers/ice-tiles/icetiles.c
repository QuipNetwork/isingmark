/* Generic tile worm for spin-ice constructions, with a CPU-time budget.
 *
 * A tile is any 4-spin set with a 16-entry energy table over its local state (bit k set when the
 * corner k is +1). A tile is "excited" when its table value is above its minimum. A worm flips a
 * seed set (one spin, a whole tile or a strong pair), then repairs every excited tile that a flip
 * touched: in the head tile it flips a corner other than the spin it entered by whose flip returns
 * the tile excitation (strictly lower table value), which excites the other tiles of that corner, and so on,
 * until no touched tile is excited (closed) or a head has no fix (failed). A tile with no single
 * corner fix (a reversed-bond defect tile, whose two ground states differ by a full flip) is
 * crossed by flipping its three other corners with probability `p_cross`; otherwise the leg ends
 * there and the excitation it leaves is charged to the worm. Among the candidate
 * corners the leg takes the lowest total flip delta (zero temperature, ties by a xorshift draw)
 * or a Boltzmann draw at `guide * beta`. A closed worm is kept when its exact energy change is
 * below zero, equal to zero within the per-call zero-move flip budget, or by a Metropolis draw at
 * `beta`. A spin with a rigid partner (`partner`, a strong pair outside the tiles such as a vertical
 * bond of stacked ice) is always flipped together with it, and the partner's tiles are repaired too.
 * `beta` is geometric in the elapsed fraction of the budget from beta0 to beta1; beta0 <= 0
 * means zero temperature throughout. Every `free_every` worms the untiled spins get one Metropolis
 * sweep at the current beta (zero temperature: only downhill flips).
 *
 * The run stops at `budget_s` of thread CPU time, at `max_worms`, or after `patience` worms with
 * no improvement at zero temperature. The best state seen is copied to `best` with its energy.
 *
 * Build: gcc -O3 -march=native -shared -fPIC -Wall -Wextra -o libicetiles.so icetiles.c
 */
#include <math.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static uint64_t xs64(uint64_t *x)
{
    *x ^= *x << 13;
    *x ^= *x >> 7;
    *x ^= *x << 17;
    return *x;
}

static double unif(uint64_t *x)
{
    return (double)(xs64(x) >> 11) * (1.0 / 9007199254740992.0);
}

static double cpu_now(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_THREAD_CPUTIME_ID, &ts);
    return (double)ts.tv_sec + 1e-9 * (double)ts.tv_nsec;
}

typedef struct {
    int32_t n;
    const int32_t *adj_ptr;
    const int32_t *adj_idx;
    const int64_t *adj_j;
    const int64_t *h;
    int32_t n_tiles;
    const int32_t *tiles;    /* T x 4 */
    const int64_t *etab;     /* T x 16, energy minus the tile minimum (0 on a ground state) */
    const int32_t *st_ptr;   /* spin -> tiles, CSR */
    const int32_t *st_idx;
    const int32_t *partner;  /* rigid partner of each spin, or -1: flipped together with it */
    const int64_t *partner_j;
    int8_t *s;
    int64_t *f;              /* local fields h_i + sum_j J_ij s_j */
    int32_t *path;
    int32_t max_len;
    int32_t *stack;          /* pairs (tile, enter spin) */
    int32_t stack_cap;
    uint64_t rng;
} ctx_t;

static void flip_spin(ctx_t *c, int32_t u)
{
    int64_t old = c->s[u];
    c->s[u] = (int8_t)(-old);
    for (int32_t q = c->adj_ptr[u]; q < c->adj_ptr[u + 1]; q++) {
        c->f[c->adj_idx[q]] -= 2 * c->adj_j[q] * old;
    }
}

static int32_t tile_state(const ctx_t *c, int32_t t)
{
    const int32_t *corner = c->tiles + 4 * t;
    int32_t idx = 0;
    for (int k = 0; k < 4; k++) {
        if (c->s[corner[k]] > 0) {
            idx |= 1 << k;
        }
    }
    return idx;
}

/* Push the other tiles of spin y (not `from`) that are excited. Returns 0 when the stack is full. */
static int push_tiles(ctx_t *c, int32_t y, int32_t from, int32_t *top)
{
    for (int32_t p = c->st_ptr[y]; p < c->st_ptr[y + 1]; p++) {
        int32_t t = c->st_idx[p];
        if (t == from) {
            continue;
        }
        if (*top + 2 > c->stack_cap) {
            return 0;
        }
        c->stack[(*top)++] = t;
        c->stack[(*top)++] = y;
    }
    return 1;
}

/* Flip delta of spin y together with its rigid partner, when it has one. */
static int64_t unit_delta(const ctx_t *c, int32_t y)
{
    int64_t d = -2 * (int64_t)c->s[y] * c->f[y];
    int32_t p = c->partner[y];
    if (p >= 0) {
        d += -2 * (int64_t)c->s[p] * c->f[p] + 4 * c->partner_j[y] * c->s[y] * c->s[p];
    }
    return d;
}

/* Flip y and its partner, record them on the path, and push their other excited tiles. */
static int flip_unit(ctx_t *c, int32_t y, int32_t from, int32_t *top, int32_t *len, int64_t *cum)
{
    *cum += -2 * (int64_t)c->s[y] * c->f[y];
    flip_spin(c, y);
    c->path[(*len)++] = y;
    int ok = push_tiles(c, y, from, top);
    int32_t p = c->partner[y];
    if (p >= 0) {
        *cum += -2 * (int64_t)c->s[p] * c->f[p];
        flip_spin(c, p);
        c->path[(*len)++] = p;
        ok = ok && push_tiles(c, p, -1, top);
    }
    return ok;
}

/* One worm from the given seed. Returns the exact energy change and the path length through
 * *len_out, or sets *closed to 0 when the worm failed (the caller undoes the path). */
static int64_t worm(ctx_t *c, const int32_t *seed, int32_t n_seed, double beta_leg, double p_cross,
                    int32_t *len_out, int *closed, int *crossed)
{
    int64_t cum = 0;
    int32_t len = 0;
    int32_t top = 0;
    *closed = 1;
    for (int32_t i = 0; i < n_seed; i++) {
        if (!flip_unit(c, seed[i], -1, &top, &len, &cum)) {
            *closed = 0;
            *len_out = len;
            return cum;
        }
    }
    while (top > 0) {
        int32_t enter = c->stack[--top];
        int32_t t = c->stack[--top];
        int32_t idx = tile_state(c, t);
        if (c->etab[16 * t + idx] == 0) {
            continue;
        }
        if (len + 8 >= c->max_len) {
            *closed = 0;
            break;
        }
        const int32_t *corner = c->tiles + 4 * t;
        const int64_t *row = c->etab + 16 * t;
        int32_t pick = -1;
        int64_t dpick = 0;
        int ties = 0;
        int64_t dk[4];
        double w[4];
        int ncand = 0;
        for (int k = 0; k < 4; k++) {
            int32_t y = corner[k];
            dk[k] = 0;
            w[k] = 0.0;
            if (y == enter || row[idx ^ (1 << k)] >= row[idx]) {
                continue;
            }
            ncand++;
            int64_t d = unit_delta(c, y);
            dk[k] = d;
            w[k] = 1.0;
            if (pick < 0 || d < dpick) {
                pick = k;
                dpick = d;
                ties = 1;
            } else if (d == dpick) {
                ties++;
                if (xs64(&c->rng) % (uint64_t)ties == 0) {
                    pick = k;
                }
            }
        }
        if (ncand == 0) {
            /* No corner lowers the tile: the leg ends here and the excitation stays (its cost is in
             * cum), or a rigid tile is crossed by flipping its three other corners with probability
             * p_cross when that reaches a ground state. */
            int32_t ek = -1;
            for (int k = 0; k < 4; k++) {
                if (corner[k] == enter) {
                    ek = k;
                }
            }
            int32_t idx3 = idx ^ (15 & ~(ek >= 0 ? (1 << ek) : 0));
            if (p_cross <= 0.0 || ek < 0 || row[idx3] != 0 || unif(&c->rng) >= p_cross) {
                continue;
            }
            (*crossed)++;
            int ok = 1;
            for (int k = 0; k < 4 && ok; k++) {
                if (k == ek) {
                    continue;
                }
                ok = flip_unit(c, corner[k], t, &top, &len, &cum);
            }
            if (!ok) {
                *closed = 0;
                break;
            }
            continue;
        }
        if (beta_leg > 0.0 && ncand > 1) {
            double tot = 0.0;
            for (int k = 0; k < 4; k++) {
                if (w[k] > 0.0) {
                    w[k] = exp(-beta_leg * (double)(dk[k] - dpick));
                    tot += w[k];
                }
            }
            double r = unif(&c->rng) * tot;
            for (int k = 0; k < 4; k++) {
                if (w[k] > 0.0) {
                    r -= w[k];
                    if (r <= 0.0) {
                        pick = k;
                        break;
                    }
                }
            }
            dpick = dk[pick];
        }
        if (!flip_unit(c, corner[pick], t, &top, &len, &cum)) {
            *closed = 0;
            break;
        }
    }
    *len_out = len;
    return cum;
}

/* stats: [0] worms, [1] accepted, [2] strictly improving, [3] failed (open), [4] crossed tiles,
 * [5] free-spin flips. Returns 0, or -1 on allocation failure. */
int worm_run(int32_t n, const int32_t *adj_ptr, const int32_t *adj_idx, const int64_t *adj_j, const int64_t *h,
             int32_t n_tiles, const int32_t *tiles, const int64_t *etab,
             const int32_t *st_ptr, const int32_t *st_idx, const int32_t *partner, const int64_t *partner_j,
             int32_t n_seeds, const int32_t *seed_ptr,
             const int32_t *seed_idx, int32_t n_free, const int32_t *free_spins, int8_t *s, int8_t *best,
             int64_t *energy_io, double budget_s, int64_t max_worms, int64_t patience, double beta0, double beta1,
             double guide, double p_cross, int32_t max_len, int64_t zero_budget, int32_t free_every,
             uint64_t seed, int64_t *stats)
{
    ctx_t c;
    memset(&c, 0, sizeof c);
    c.n = n;
    c.adj_ptr = adj_ptr;
    c.adj_idx = adj_idx;
    c.adj_j = adj_j;
    c.h = h;
    c.n_tiles = n_tiles;
    c.tiles = tiles;
    c.etab = etab;
    c.st_ptr = st_ptr;
    c.st_idx = st_idx;
    c.partner = partner;
    c.partner_j = partner_j;
    c.s = s;
    c.max_len = max_len;
    c.stack_cap = 8 * max_len + 64;
    c.rng = seed ? seed : 0x9e3779b97f4a7c15ULL;
    c.f = malloc(sizeof(int64_t) * (size_t)n);
    c.path = malloc(sizeof(int32_t) * (size_t)max_len);
    c.stack = malloc(sizeof(int32_t) * (size_t)c.stack_cap);
    if (c.f == NULL || c.path == NULL || c.stack == NULL) {
        free(c.f);
        free(c.path);
        free(c.stack);
        return -1;
    }
    for (int32_t i = 0; i < n; i++) {
        int64_t acc = h[i];
        for (int32_t k = adj_ptr[i]; k < adj_ptr[i + 1]; k++) {
            acc += adj_j[k] * s[adj_idx[k]];
        }
        c.f[i] = acc;
    }
    int64_t e = energy_io[0];
    int64_t e_best = e;
    memcpy(best, s, (size_t)n);
    int zero_t = beta0 <= 0.0;
    double t0 = cpu_now();
    double beta = zero_t ? 0.0 : beta0;
    double lratio = zero_t ? 0.0 : log(beta1 / beta0);
    int64_t budget = zero_budget;
    int64_t since_improve = 0;
    int crossed = 0;
    for (int64_t w = 0; w < max_worms; w++) {
        if ((w & 63) == 0) {
            double frac = (cpu_now() - t0) / budget_s;
            if (frac >= 1.0) {
                break;
            }
            if (!zero_t) {
                beta = beta0 * exp(lratio * frac);
            }
        }
        if (zero_t && patience > 0 && since_improve >= patience) {
            break;
        }
        int32_t k = (int32_t)(xs64(&c.rng) % (uint64_t)n_seeds);
        int32_t len = 0;
        int closed = 1;
        int64_t cum = worm(&c, seed_idx + seed_ptr[k], seed_ptr[k + 1] - seed_ptr[k], zero_t ? 0.0 : guide * beta,
                           p_cross, &len, &closed, &crossed);
        int keep = 0;
        if (closed) {
            if (cum < 0) {
                keep = 1;
            } else if (cum == 0) {
                keep = !zero_t || budget >= len;
                if (keep && zero_t) {
                    budget -= len;
                }
            } else if (!zero_t) {
                keep = unif(&c.rng) < exp(-beta * (double)cum);
            }
        }
        stats[0]++;
        since_improve++;
        if (keep) {
            stats[1]++;
            e += cum;
            if (cum < 0) {
                stats[2]++;
            }
            if (e < e_best) {
                e_best = e;
                memcpy(best, s, (size_t)n);
                since_improve = 0;
            }
        } else {
            if (!closed) {
                stats[3]++;
            }
            for (int32_t i = len - 1; i >= 0; i--) {
                flip_spin(&c, c.path[i]);
            }
        }
        if (free_every > 0 && n_free > 0 && (w % free_every) == 0) {
            for (int32_t i = 0; i < n_free; i++) {
                int32_t u = free_spins[i];
                int64_t d = -2 * (int64_t)s[u] * c.f[u];
                if (d < 0 || (d > 0 && !zero_t && unif(&c.rng) < exp(-beta * (double)d))) {
                    flip_spin(&c, u);
                    e += d;
                    stats[5]++;
                }
            }
            if (e < e_best) {
                e_best = e;
                memcpy(best, s, (size_t)n);
                since_improve = 0;
            }
        }
    }
    stats[4] += crossed;
    energy_io[0] = e;
    energy_io[1] = e_best;
    free(c.f);
    free(c.path);
    free(c.stack);
    return 0;
}
