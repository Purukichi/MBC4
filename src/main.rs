//! Standalone app (`cargo truce run`), no DAW needed. Only compiled with the
//! `standalone` feature (see `[[bin]]` in Cargo.toml).

use mbc4::Plugin;

fn main() {
    truce_standalone::run::<Plugin>();
}
