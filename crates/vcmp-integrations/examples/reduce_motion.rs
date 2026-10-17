//! Prints the operating system's reduce-motion setting as VCMP reads it: "reduce",
//! "don't reduce", or "unknown" where the system has none. Then it prints the setting
//! again each time it changes, until stopped with Ctrl+C. On macOS, changes arrive
//! through the main run loop, which only the app runs, so there it prints the
//! setting at start only.
//!
//!     cargo run -p vcmp-integrations --example reduce_motion

use vcmp_integrations::reduce_motion;

fn main() {
    if reduce_motion::system_setting().is_none() {
        println!("unknown");
        return;
    }
    reduce_motion::follow(|reduce| println!("{}", if reduce { "reduce" } else { "don't reduce" }));
    loop {
        std::thread::park();
    }
}
