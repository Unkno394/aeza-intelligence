//! Online behavioral history for Aéza customer signals.
//!
//! The model evaluates an observation against the profile accumulated before
//! that observation, then updates the profile. It reports behavioral novelty
//! and deviation; it does not interpret either as churn on its own.

pub mod historical;
