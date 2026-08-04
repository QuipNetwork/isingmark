// SPDX-License-Identifier: AGPL-3.0-or-later
//! `IsingMark` CLI entry point.

fn main() {
    #[expect(
        clippy::print_stdout,
        reason = "single documented CLI output boundary for the version banner"
    )]
    {
        println!("isingmark {}", env!("CARGO_PKG_VERSION"));
    }
}
