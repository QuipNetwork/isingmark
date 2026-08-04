// SPDX-License-Identifier: AGPL-3.0-or-later
#[test]
fn draw_ising_milli_is_reachable_and_deterministic() {
    let nonce = [7u8; 32];
    let (h1, j1) =
        quip_protocol::chacha8::draw_ising_milli(nonce, 8, 8, &[-1000, 0, 1000], &[-1000, 1000])
            .expect("draw succeeds for non-empty allowed sets");
    let (h2, j2) =
        quip_protocol::chacha8::draw_ising_milli(nonce, 8, 8, &[-1000, 0, 1000], &[-1000, 1000])
            .expect("draw succeeds for non-empty allowed sets");
    assert_eq!(h1.len(), 8);
    assert_eq!(j1.len(), 8);
    assert_eq!((h1, j1), (h2, j2), "same nonce must yield the same problem");
}
