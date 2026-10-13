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
#[cfg(test)]
mod t03_burn;
#[cfg(test)]
mod t04_vault_share;
#[cfg(test)]
mod t05_king;
#[cfg(test)]
mod t06_graduated;
#[cfg(test)]
mod t07_math;
#[cfg(test)]
mod t08_readers;
