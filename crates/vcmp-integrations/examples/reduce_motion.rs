//! Prints the operating system's reduce-motion setting as VCMP reads it at start:
//! "reduce", "don't reduce", or "unknown" where the system has none.
//!
//!     cargo run -p vcmp-integrations --example reduce_motion

use vcmp_integrations::reduce_motion;

fn main() {
    let answer = match reduce_motion::system_setting() {
        Some(true) => "reduce",
        Some(false) => "don't reduce",
        None => "unknown",
    };
    println!("{answer}");
}
