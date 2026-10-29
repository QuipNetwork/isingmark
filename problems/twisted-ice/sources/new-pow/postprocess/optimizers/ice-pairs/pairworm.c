/* Pair-aware worm descent and min-cost defect repair for twisted ice with planted pairs.
 *
 * The model is the gauged K4 tile picture of greedy.c (loop_descent): in the gauge frame a tile has
 * charge q = sum of g s over its four corners, the ice manifold is q = 0 on every tile. A planted
 * pair (u, v) is a ferromagnetic coupler, satisfied when s_u = s_v. The moves here keep the pairs
 * satisfied by flipping both ends as one unit, which spawns extra charged tiles at the partner. A
 * worm is therefore a branching walk: a stack of charged tiles, each resolved by flipping one more
 * corner, until no tile touched by the walk carries charge. The walk is undone unless its exact
 * energy change is negative (or zero within a per-read flip budget).
 *
 * Build: gcc -O3 -march=native -shared -fPIC -Wall -Wextra -o libpairworm.so pairworm.c
 */
#include <math.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define STACK_SAVE 64

typedef struct {
    int32_t n;
    const int32_t *adj_ptr;
    const int32_t *adj_idx;
    const int64_t *adj_j;
    const int64_t *h;
    const int32_t *tiles;   /* T x 4 corner spins */
    const int32_t *gauge;   /* T x 4 gauge signs */
    const int32_t *mtile;   /* n x 2 tiles per spin, -1 when absent */
    int32_t n_tiles;
    const int32_t *partner; /* n, pair partner or -1 */
} Model;

typedef struct {
    int8_t *s;
    int64_t *f;       /* local fields h_i + sum_j J_ij s_j */
    int8_t *inpath;   /* spin flipped by the current worm */
    int32_t *path;    /* flipped spins in order */
    int32_t len;
    int32_t *stack;   /* tiles to check */
    int32_t sp;
    int64_t cum;      /* exact energy change so far */
    int32_t max_len;
    double temp;      /* Metropolis temperature in milli, 0 for strict descent */
    uint64_t rng;
} Walk;

static uint64_t xs64(uint64_t *x)
{
    *x ^= *x << 13;
    *x ^= *x >> 7;
    *x ^= *x << 17;
    return *x;
}

static void fields(const Model *m, const int8_t *s, int64_t *f)
{
    for (int32_t i = 0; i < m->n; i++) {
        int64_t acc = m->h[i];
        for (int32_t k = m->adj_ptr[i]; k < m->adj_ptr[i + 1]; k++) {
            acc += m->adj_j[k] * s[m->adj_idx[k]];
        }
        f[i] = acc;
    }
}

static void flip(const Model *m, int8_t *s, int64_t *f, int32_t u)
{
    int64_t old = s[u];
    s[u] = (int8_t)(-old);
    for (int32_t q = m->adj_ptr[u]; q < m->adj_ptr[u + 1]; q++) {
        f[m->adj_idx[q]] -= 2 * m->adj_j[q] * old;
    }
}

static int32_t charge(const Model *m, const int8_t *s, int32_t t)
{
    int32_t q = 0;
    for (int k = 0; k < 4; k++) {
        q += m->gauge[4 * t + k] * s[m->tiles[4 * t + k]];
    }
    return q;
}

static void push_tiles(const Model *m, Walk *w, int32_t u)
{
    for (int k = 0; k < 2; k++) {
        int32_t t = m->mtile[2 * u + k];
        if (t >= 0) {
            w->stack[w->sp++] = t;
        }
    }
}

/* Flip u as part of the walk, then its partner when the pair would otherwise break. */
static void walk_flip(const Model *m, Walk *w, int32_t u)
{
    w->cum += -2 * (int64_t)w->s[u] * w->f[u];
    flip(m, w->s, w->f, u);
    w->inpath[u] = 1;
    w->path[w->len++] = u;
    push_tiles(m, w, u);
    int32_t p = m->partner[u];
    if (p >= 0 && !w->inpath[p] && w->s[u] != w->s[p]) {
        w->cum += -2 * (int64_t)w->s[p] * w->f[p];
        flip(m, w->s, w->f, p);
        w->inpath[p] = 1;
        w->path[w->len++] = p;
        push_tiles(m, w, p);
    }
}

/* Energy change of flipping y and, when the pair would break, its partner. */
static int64_t unit_delta(const Model *m, const Walk *w, int32_t y)
{
    int64_t d = -2 * (int64_t)w->s[y] * w->f[y];
    int32_t p = m->partner[y];
    if (p >= 0 && !w->inpath[p] && w->s[y] == w->s[p]) {
        /* both flip: delta_y + delta_p + 4 J_yp s_y s_p, and J_yp is the pair coupler */
        int64_t jyp = 0;
        for (int32_t q = m->adj_ptr[y]; q < m->adj_ptr[y + 1]; q++) {
            if (m->adj_idx[q] == p) {
                jyp = m->adj_j[q];
                break;
            }
        }
        d += -2 * (int64_t)w->s[p] * w->f[p] + 4 * jyp * w->s[y] * w->s[p];
    }
    return d;
}

static void undo(const Model *m, Walk *w)
{
    for (int32_t k = w->len - 1; k >= 0; k--) {
        flip(m, w->s, w->f, w->path[k]);
        w->inpath[w->path[k]] = 0;
    }
    w->len = 0;
    w->sp = 0;
    w->cum = 0;
}

static void commit(Walk *w)
{
    for (int32_t k = 0; k < w->len; k++) {
        w->inpath[w->path[k]] = 0;
    }
    w->len = 0;
    w->sp = 0;
    w->cum = 0;
}

/* Run the walk from the first flip of u0 until every touched tile is neutral. Returns 1 when the
 * walk closed (the caller decides on acceptance), 0 when it failed (already undone). */
static int worm(const Model *m, Walk *w, int32_t u0)
{
    walk_flip(m, w, u0);
    while (w->sp > 0) {
        int32_t t = w->stack[--w->sp];
        int32_t q = charge(m, w->s, t);
        if (q == 0) {
            continue;
        }
        if (w->len >= w->max_len) {
            undo(m, w);
            return 0;
        }
        int32_t sg = q > 0 ? 1 : -1;
        int32_t pick = -1;
        int64_t dpick = 0;
        int ties = 0;
        for (int k = 0; k < 4; k++) {
            int32_t y = m->tiles[4 * t + k];
            if (w->inpath[y] || m->gauge[4 * t + k] * w->s[y] != sg) {
                continue;
            }
            int64_t d = unit_delta(m, w, y);
            if (pick < 0 || d < dpick) {
                pick = y;
                dpick = d;
                ties = 1;
            } else if (d == dpick) {
                ties++;
                if (xs64(&w->rng) % (uint64_t)ties == 0) {
                    pick = y;
                }
            }
        }
        if (pick < 0) {
            undo(m, w);
            return 0;
        }
        w->stack[w->sp++] = t; /* re-check: a charge of 4 needs two flips */
        walk_flip(m, w, pick);
    }
    return 1;
}

/* Metropolis draw for an uphill walk: accept with probability exp(-cum / temp). */
static int accept_uphill(Walk *w)
{
    if (w->temp <= 0 || w->cum <= 0) {
        return 0;
    }
    double u = (double)(xs64(&w->rng) >> 11) * (1.0 / 9007199254740992.0);
    return u < exp(-(double)w->cum / w->temp);
}

/* Try a worm from u0; keep it when it lowers the energy, or keeps it within the zero budget. */
static int try_worm(const Model *m, Walk *w, int32_t u0, int64_t *budget, int *improved)
{
    if (!worm(m, w, u0)) {
        return 0;
    }
    if (w->cum < 0) {
        *improved = 1;
        commit(w);
        return 1;
    }
    if ((w->cum == 0 && *budget >= w->len) || accept_uphill(w)) {
        if (w->cum == 0) {
            *budget -= w->len;
        }
        commit(w);
        return 1;
    }
    undo(m, w);
    return 0;
}

/* Exhaustive depth-limited worm: at every charged tile branch over the candidate corners (cheapest
 * unit delta first) and backtrack, accepting the first closed walk with a negative exact change (or
 * zero within the budget). `prune` cuts a branch whose running change exceeds it. The stack segment
 * above the first charged tile is saved and restored around the branch, because pushes overwrite
 * it. */
static int dfs(const Model *m, Walk *w, int depth_left, int64_t prune, int64_t *budget, int *improved)
{
    int32_t sp0 = w->sp;
    int32_t t = -1;
    while (w->sp > 0) {
        t = w->stack[w->sp - 1];
        if (charge(m, w->s, t) != 0) {
            break;
        }
        w->sp--;
        t = -1;
    }
    if (t < 0) {
        if (w->cum < 0) {
            *improved = 1;
            return 1;
        }
        if ((w->cum == 0 && *budget >= w->len) || accept_uphill(w)) {
            if (w->cum == 0) {
                *budget -= w->len;
            }
            return 1;
        }
        w->sp = sp0;
        return 0;
    }
    if (depth_left == 0 || w->len >= w->max_len || w->cum > prune) {
        w->sp = sp0;
        return 0;
    }
    int32_t nsave = sp0 - w->sp;
    int32_t save[STACK_SAVE];
    if (nsave > STACK_SAVE) {
        w->sp = sp0;
        return 0;
    }
    memcpy(save, &w->stack[w->sp], sizeof(int32_t) * (size_t)nsave);
    int32_t q = charge(m, w->s, t);
    int32_t sg = q > 0 ? 1 : -1;
    int32_t cand[4];
    int64_t cd[4];
    int nc = 0;
    for (int k = 0; k < 4; k++) {
        int32_t y = m->tiles[4 * t + k];
        if (w->inpath[y] || m->gauge[4 * t + k] * w->s[y] != sg) {
            continue;
        }
        cand[nc] = y;
        cd[nc] = unit_delta(m, w, y);
        nc++;
    }
    for (int a = 1; a < nc; a++) {
        for (int b = a; b > 0 && cd[b] < cd[b - 1]; b--) {
            int64_t td = cd[b];
            cd[b] = cd[b - 1];
            cd[b - 1] = td;
            int32_t ty = cand[b];
            cand[b] = cand[b - 1];
            cand[b - 1] = ty;
        }
    }
    for (int a = 0; a < nc; a++) {
        int32_t len0 = w->len;
        int64_t cum0 = w->cum;
        int32_t sp1 = w->sp;
        walk_flip(m, w, cand[a]);
        if (dfs(m, w, depth_left - 1, prune, budget, improved)) {
            return 1;
        }
        for (int32_t k = w->len - 1; k >= len0; k--) {
            flip(m, w->s, w->f, w->path[k]);
            w->inpath[w->path[k]] = 0;
        }
        w->len = len0;
        w->cum = cum0;
        w->sp = sp1;
    }
    memcpy(&w->stack[w->sp], save, sizeof(int32_t) * (size_t)nsave);
    w->sp = sp0;
    return 0;
}

/* Try a worm from u0 with the exhaustive walk when depth > 0, else the greedy walk. */
static int try_move(const Model *m, Walk *w, int32_t u0, int depth, int64_t prune, int64_t *budget, int *improved)
{
    if (depth <= 0) {
        return try_worm(m, w, u0, budget, improved);
    }
    walk_flip(m, w, u0);
    if (dfs(m, w, depth, prune, budget, improved)) {
        commit(w);
        return 1;
    }
    undo(m, w);
    return 0;
}

/* Pair-aware worm descent on `reads` states. Each sweep runs a defect pass (one worm attempt from
 * every corner of every charged tile and from both ends of every broken pair, cheapest unit delta
 * first) and, when `random_pass` is set, a pass from every tiled spin in random order. The defect
 * pass uses the exhaustive walk to `defect_depth` units, the random pass to `random_depth` (0 for
 * the greedy walk). With `temp` above zero uphill walks are kept with the Metropolis probability
 * and every sweep runs; otherwise sweeps stop after one with no strict improvement, or at max_sweeps. Returns accepted worms, -1 on allocation
 * failure. */
int64_t pair_worm_descent(int32_t n, const int32_t *adj_ptr, const int32_t *adj_idx, const int64_t *adj_j,
                          const int64_t *h, const int32_t *tiles, const int32_t *gauge, const int32_t *mtile,
                          int32_t n_tiles, const int32_t *partner, int32_t n_tiled, const int32_t *tiled,
                          const int32_t *pairs, int32_t n_pairs, int32_t reads, int8_t *spins, int32_t max_len,
                          int32_t max_sweeps, int64_t zero_budget, int random_pass, int defect_depth,
                          int random_depth, int64_t prune, double temp, uint64_t seed)
{
    Model m = {n, adj_ptr, adj_idx, adj_j, h, tiles, gauge, mtile, n_tiles, partner};
    Walk w;
    int64_t accepted = 0;
    w.f = malloc(sizeof(int64_t) * (size_t)n);
    w.inpath = calloc((size_t)n, 1);
    w.path = malloc(sizeof(int32_t) * (size_t)(max_len + 2));
    w.stack = malloc(sizeof(int32_t) * (size_t)(6 * (max_len + 2) + 8));
    int32_t *order = malloc(sizeof(int32_t) * (size_t)(n_tiled > 0 ? n_tiled : 1));
    if (w.f == NULL || w.inpath == NULL || w.path == NULL || w.stack == NULL || order == NULL) {
        free(w.f);
        free(w.inpath);
        free(w.path);
        free(w.stack);
        free(order);
        return -1;
    }
    w.max_len = max_len;
    w.temp = temp;
    w.rng = seed ? seed : 0x9e3779b97f4a7c15ULL;
    for (int32_t r = 0; r < reads; r++) {
        w.s = spins + (size_t)r * (size_t)n;
        w.len = 0;
        w.sp = 0;
        w.cum = 0;
        int64_t budget = zero_budget;
        fields(&m, w.s, w.f);
        memcpy(order, tiled, sizeof(int32_t) * (size_t)n_tiled);
        for (int32_t sweep = 0; sweep < max_sweeps; sweep++) {
            int improved = 0;
            /* defect pass: charged tiles */
            for (int32_t t = 0; t < n_tiles; t++) {
                int32_t q = charge(&m, w.s, t);
                if (q == 0) {
                    continue;
                }
                int32_t sg = q > 0 ? 1 : -1;
                int32_t cand[4];
                int64_t cd[4];
                int nc = 0;
                for (int k = 0; k < 4; k++) {
                    int32_t y = tiles[4 * t + k];
                    if (gauge[4 * t + k] * w.s[y] == sg) {
                        cand[nc] = y;
                        cd[nc] = unit_delta(&m, &w, y);
                        nc++;
                    }
                }
                /* insertion sort by delta */
                for (int a = 1; a < nc; a++) {
                    for (int b = a; b > 0 && cd[b] < cd[b - 1]; b--) {
                        int64_t td = cd[b];
                        cd[b] = cd[b - 1];
                        cd[b - 1] = td;
                        int32_t ty = cand[b];
                        cand[b] = cand[b - 1];
                        cand[b - 1] = ty;
                    }
                }
                for (int a = 0; a < nc; a++) {
                    if (try_move(&m, &w, cand[a], defect_depth, prune, &budget, &improved)) {
                        accepted++;
                        break;
                    }
                }
            }
            /* defect pass: broken pairs */
            for (int32_t p = 0; p < n_pairs; p++) {
                int32_t u = pairs[2 * p];
                int32_t v = pairs[2 * p + 1];
                if (w.s[u] == w.s[v]) {
                    continue;
                }
                if (try_move(&m, &w, u, defect_depth, prune, &budget, &improved)
                    || try_move(&m, &w, v, defect_depth, prune, &budget, &improved)) {
                    accepted++;
                }
            }
            if (random_pass) {
                for (int32_t i = n_tiled - 1; i > 0; i--) {
                    int32_t k = (int32_t)(xs64(&w.rng) % (uint64_t)(i + 1));
                    int32_t tmp = order[i];
                    order[i] = order[k];
                    order[k] = tmp;
                }
                for (int32_t oi = 0; oi < n_tiled; oi++) {
                    if (try_move(&m, &w, order[oi], random_depth, prune, &budget, &improved)) {
                        accepted++;
                    }
                }
            }
            if (!improved && temp <= 0) {
                break;
            }
        }
    }
    free(w.f);
    free(w.inpath);
    free(w.path);
    free(w.stack);
    free(order);
    return accepted;
}

/* Count of charged tiles and broken pairs in each read: out[2 r] tiles, out[2 r + 1] pairs. */
void defect_counts(int32_t n, const int32_t *tiles, const int32_t *gauge, int32_t n_tiles, const int32_t *pairs,
                   int32_t n_pairs, int32_t reads, const int8_t *spins, int32_t *out)
{
    for (int32_t r = 0; r < reads; r++) {
        const int8_t *s = spins + (size_t)r * (size_t)n;
        int32_t ct = 0;
        int32_t cp = 0;
        for (int32_t t = 0; t < n_tiles; t++) {
            int32_t q = 0;
            for (int k = 0; k < 4; k++) {
                q += gauge[4 * t + k] * s[tiles[4 * t + k]];
            }
            ct += q != 0;
        }
        for (int32_t p = 0; p < n_pairs; p++) {
            cp += s[pairs[2 * p]] != s[pairs[2 * p + 1]];
        }
        out[2 * r] = ct;
        out[2 * r + 1] = cp;
    }
}

/* Read combination: flip into the reference state `s` every connected block on which another state
 * differs from it, when the exact energy change of that block flip is negative. Blocks are
 * connected components of the difference set over the edges of `cadj` (the strong couplers, so a
 * block carries whole tiles and pairs and the flip keeps a manifold state in the manifold). Each
 * other state is first sign-aligned to the reference when `align` is set (the models have no
 * fields, so s and -s are the same state). Returns the number of blocks flipped, -1 on allocation
 * failure. */
int64_t crossover(int32_t n, const int32_t *adj_ptr, const int32_t *adj_idx, const int64_t *adj_j, const int64_t *h,
                  const int32_t *cadj_ptr, const int32_t *cadj_idx, int8_t *s, int32_t others, const int8_t *pool,
                  int align, int32_t min_size, int32_t max_size)
{
    int64_t *f = malloc(sizeof(int64_t) * (size_t)n);
    int8_t *diff = malloc((size_t)n);
    int8_t *mark = malloc((size_t)n); /* 0 unseen, 1 in the current block, 2 finished */
    int32_t *queue = malloc(sizeof(int32_t) * (size_t)n);
    int64_t flipped = 0;
    if (f == NULL || diff == NULL || mark == NULL || queue == NULL) {
        free(f);
        free(diff);
        free(mark);
        free(queue);
        return -1;
    }
    for (int32_t i = 0; i < n; i++) {
        int64_t acc = h[i];
        for (int32_t k = adj_ptr[i]; k < adj_ptr[i + 1]; k++) {
            acc += adj_j[k] * s[adj_idx[k]];
        }
        f[i] = acc;
    }
    for (int32_t r = 0; r < others; r++) {
        const int8_t *b = pool + (size_t)r * (size_t)n;
        int8_t sign = 1;
        if (align) {
            int64_t dot = 0;
            for (int32_t i = 0; i < n; i++) {
                dot += (int64_t)s[i] * b[i];
            }
            sign = dot < 0 ? -1 : 1;
        }
        for (int32_t i = 0; i < n; i++) {
            diff[i] = (int8_t)(s[i] != sign * b[i]);
            mark[i] = 0;
        }
        for (int32_t root = 0; root < n; root++) {
            if (!diff[root] || mark[root]) {
                continue;
            }
            int32_t head = 0;
            int32_t tail = 0;
            queue[tail++] = root;
            mark[root] = 1;
            while (head < tail) {
                int32_t u = queue[head++];
                for (int32_t k = cadj_ptr[u]; k < cadj_ptr[u + 1]; k++) {
                    int32_t v = cadj_idx[k];
                    if (diff[v] && !mark[v]) {
                        mark[v] = 1;
                        queue[tail++] = v;
                    }
                }
            }
            if (tail >= min_size && tail <= max_size) {
                int64_t d = 0;
                for (int32_t q = 0; q < tail; q++) {
                    int32_t u = queue[q];
                    d += -2 * (int64_t)s[u] * f[u];
                    for (int32_t k = adj_ptr[u]; k < adj_ptr[u + 1]; k++) {
                        int32_t v = adj_idx[k];
                        if (v > u && mark[v] == 1) {
                            d += 4 * adj_j[k] * s[u] * s[v];
                        }
                    }
                }
                if (d < 0) {
                    for (int32_t q = 0; q < tail; q++) {
                        int64_t old = s[queue[q]];
                        s[queue[q]] = (int8_t)(-old);
                        for (int32_t k = adj_ptr[queue[q]]; k < adj_ptr[queue[q] + 1]; k++) {
                            f[adj_idx[k]] -= 2 * adj_j[k] * old;
                        }
                    }
                    flipped++;
                }
            }
            for (int32_t q = 0; q < tail; q++) {
                mark[queue[q]] = 2;
            }
        }
    }
    free(f);
    free(diff);
    free(mark);
    free(queue);
    return flipped;
}
