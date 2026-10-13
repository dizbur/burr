//! Test bench. Runs Burr against real copies of pump.fun's programs, taken
//! from the main network by reading (see `scripts/ground/07-fetch-fixtures.mjs`).

pub mod bench;
pub mod kit;
pub mod pump;

#[cfg(test)]
mod t00_pump_alone;
#[cfg(test)]
mod t01_frame;
#[cfg(test)]
mod t02_collect_split;
