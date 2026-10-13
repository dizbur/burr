//! Test bench. Runs Burrpad against real copies of pump.fun's programs, taken
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
mod t04_holders;
#[cfg(test)]
mod t06_graduated;
#[cfg(test)]
mod t07_math;
#[cfg(test)]
mod t08_readers;
#[cfg(test)]
mod t09_misbehaving_pump;
#[cfg(test)]
mod t10_whole_life;
#[cfg(test)]
mod t11_substitution;
#[cfg(test)]
mod t12_pump_facts;
#[cfg(test)]
mod t13_launch_with_buy;
