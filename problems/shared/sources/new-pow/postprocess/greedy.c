/* First-improvement greedy descent over a move set, one read at a time, with incremental local fields.
 *
 * A move is a set of spins. Its energy change is sum over members of -2 s_i f_i plus 4 J_ab s_a s_b
 * over its internal edges, where f_i = h_i + sum_j J_ij s_j. The sweep visits the moves in order and
 * applies every move that lowers the energy, until a full sweep applies none.
 *
 * Build: gcc -O3 -march=native -shared -fPIC -Wall -Wextra -o libgreedy.so greedy.c
 */
#include <stdint.h>
#include <stdlib.h>

int64_t greedy_descent(int32_t n, const int32_t *adj_ptr, const int32_t *adj_idx, const int64_t *adj_j,
                       const int64_t *h, int32_t n_moves, const int32_t *mv_ptr, const int32_t *mv_idx,
                       const int32_t *in_ptr, const int32_t *in_a, const int32_t *in_b, const int64_t *in_j,
                       int32_t reads, int8_t *spins)
{
    int64_t *f = malloc(sizeof(int64_t) * (size_t)n);
    int64_t applied = 0;
    if (f == NULL) {
        return -1;
    }
    for (int32_t r = 0; r < reads; r++) {
        int8_t *s = spins + (size_t)r * (size_t)n;
        for (int32_t i = 0; i < n; i++) {
            int64_t acc = h[i];
            for (int32_t k = adj_ptr[i]; k < adj_ptr[i + 1]; k++) {
                acc += adj_j[k] * s[adj_idx[k]];
            }
            f[i] = acc;
        }
        int changed = 1;
        while (changed) {
            changed = 0;
            for (int32_t m = 0; m < n_moves; m++) {
                int64_t d = 0;
                for (int32_t k = mv_ptr[m]; k < mv_ptr[m + 1]; k++) {
                    int32_t i = mv_idx[k];
                    d += -2 * (int64_t)s[i] * f[i];
                }
                for (int32_t k = in_ptr[m]; k < in_ptr[m + 1]; k++) {
                    d += 4 * in_j[k] * s[in_a[k]] * s[in_b[k]];
                }
                if (d >= 0) {
                    continue;
                }
                for (int32_t k = mv_ptr[m]; k < mv_ptr[m + 1]; k++) {
                    int32_t i = mv_idx[k];
                    int64_t old = s[i];
                    s[i] = (int8_t)(-old);
                    for (int32_t q = adj_ptr[i]; q < adj_ptr[i + 1]; q++) {
                        f[adj_idx[q]] -= 2 * adj_j[q] * old;
                    }
                }
                applied++;
                changed = 1;
            }
        }
    }
    free(f);
    return applied;
}

/* Zero-temperature worm (loop) descent in the ice manifold of gauged K4 tiles, ported from
 * gpu/src/gpuloop.cu. In the gauge frame sigma = g s a tile has charge q = sum sigma. A worm flips a
 * tiled spin u0, then each charged end tile flips a corner y != the entering spin with
 * sigma(y) = sign(q), which moves the charge to the other tile of y, until the head charge is 0 or y
 * has no other tile. Among the candidates the leg takes the one with the lowest flip delta (ties
 * by a xorshift draw), the zero-temperature limit of the kernel's guided walk. A closed worm is kept
 * when its exact energy change is below zero, or equal to zero while the read's zero-move flip budget
 * lasts. Otherwise it is undone. Sweeps visit every tiled spin in a fresh random order and stop
 * after a sweep with no strictly improving worm, or at max_sweeps.
 *
 * mtile, mgauge: n x 2 (tile -1 when absent). Returns the accepted worms, or -1 on allocation failure.
 */
static uint64_t xs64(uint64_t *x)
{
    *x ^= *x << 13;
    *x ^= *x >> 7;
    *x ^= *x << 17;
    return *x;
}

static void flip_spin(int8_t *s, int64_t *f, int32_t u, const int32_t *adj_ptr, const int32_t *adj_idx,
                      const int64_t *adj_j)
{
    int64_t old = s[u];
    s[u] = (int8_t)(-old);
    for (int32_t q = adj_ptr[u]; q < adj_ptr[u + 1]; q++) {
        f[adj_idx[q]] -= 2 * adj_j[q] * old;
    }
}

static int32_t tile_charge(const int8_t *s, const int32_t *tiles, const int32_t *gauge, int32_t t)
{
    int32_t q = 0;
    for (int k = 0; k < 4; k++) {
        q += gauge[4 * t + k] * s[tiles[4 * t + k]];
    }
    return q;
}

int64_t loop_descent(int32_t n, const int32_t *adj_ptr, const int32_t *adj_idx, const int64_t *adj_j,
                     const int64_t *h, const int32_t *tiles, const int32_t *gauge, const int32_t *mtile,
                     int32_t n_tiled, const int32_t *tiled, int32_t reads, int8_t *spins, int32_t max_len,
                     int32_t max_sweeps, int64_t zero_budget, uint64_t seed)
{
    int64_t *f = malloc(sizeof(int64_t) * (size_t)n);
    int32_t *path = malloc(sizeof(int32_t) * (size_t)max_len);
    int32_t *order = malloc(sizeof(int32_t) * (size_t)(n_tiled > 0 ? n_tiled : 1));
    int64_t accepted = 0;
    uint64_t rng = seed ? seed : 0x9e3779b97f4a7c15ULL;
    if (f == NULL || path == NULL || order == NULL) {
        free(f);
        free(path);
        free(order);
        return -1;
    }
    for (int32_t r = 0; r < reads; r++) {
        int8_t *s = spins + (size_t)r * (size_t)n;
        int64_t budget = zero_budget;
        for (int32_t i = 0; i < n; i++) {
            int64_t acc = h[i];
            for (int32_t k = adj_ptr[i]; k < adj_ptr[i + 1]; k++) {
                acc += adj_j[k] * s[adj_idx[k]];
            }
            f[i] = acc;
        }
        for (int32_t i = 0; i < n_tiled; i++) {
            order[i] = tiled[i];
        }
        for (int32_t sweep = 0; sweep < max_sweeps; sweep++) {
            int improved = 0;
            for (int32_t i = n_tiled - 1; i > 0; i--) {
                int32_t k = (int32_t)(xs64(&rng) % (uint64_t)(i + 1));
                int32_t tmp = order[i];
                order[i] = order[k];
                order[k] = tmp;
            }
            for (int32_t oi = 0; oi < n_tiled; oi++) {
                int32_t u0 = order[oi];
                int64_t cum = -2 * (int64_t)s[u0] * f[u0];
                flip_spin(s, f, u0, adj_ptr, adj_idx, adj_j);
                int32_t len = 0;
                path[len++] = u0;
                int32_t ends[2] = {mtile[2 * u0], mtile[2 * u0 + 1]};
                if (ends[1] >= 0 && (xs64(&rng) & 1)) {
                    int32_t t = ends[0];
                    ends[0] = ends[1];
                    ends[1] = t;
                }
                int closed = 1;
                for (int leg = 0; leg < 2 && closed; leg++) {
                    int32_t head = ends[leg];
                    int32_t enter = u0;
                    if (head < 0) {
                        continue;
                    }
                    for (;;) {
                        int32_t q = tile_charge(s, tiles, gauge, head);
                        if (q == 0) {
                            break;
                        }
                        if (len >= max_len) {
                            closed = 0;
                            break;
                        }
                        int32_t sg = q > 0 ? 1 : -1;
                        int32_t pick = -1;
                        int64_t dpick = 0;
                        int ties = 0;
                        for (int k = 0; k < 4; k++) {
                            int32_t y = tiles[4 * head + k];
                            if (y == enter || gauge[4 * head + k] * s[y] != sg) {
                                continue;
                            }
                            int64_t d = -2 * (int64_t)s[y] * f[y];
                            if (pick < 0 || d < dpick) {
                                pick = y;
                                dpick = d;
                                ties = 1;
                            } else if (d == dpick) {
                                ties++;
                                if (xs64(&rng) % (uint64_t)ties == 0) {
                                    pick = y;
                                }
                            }
                        }
                        if (pick < 0) {
                            closed = 0;
                            break;
                        }
                        cum += dpick;
                        flip_spin(s, f, pick, adj_ptr, adj_idx, adj_j);
                        path[len++] = pick;
                        int32_t a = mtile[2 * pick];
                        int32_t b = mtile[2 * pick + 1];
                        int32_t other = a == head ? b : a;
                        if (other < 0) {
                            break;
                        }
                        head = other;
                        enter = pick;
                    }
                }
                int keep = closed && (cum < 0 || (cum == 0 && budget >= len));
                if (keep) {
                    accepted++;
                    if (cum < 0) {
                        improved = 1;
                    } else {
                        budget -= len;
                    }
                } else {
                    for (int32_t k = len - 1; k >= 0; k--) {
                        flip_spin(s, f, path[k], adj_ptr, adj_idx, adj_j);
                    }
                }
            }
            if (!improved) {
                break;
            }
        }
    }
    free(f);
    free(path);
    free(order);
    return accepted;
}
