
//! Autogenerated weights for `pallet_block_reward`
//!
//! THIS FILE WAS AUTO-GENERATED USING THE SUBSTRATE BENCHMARK CLI VERSION 4.0.0-dev
//! DATE: 2022-09-29, STEPS: `20`, REPEAT: 10, LOW RANGE: `[]`, HIGH RANGE: `[]`
//! EXECUTION: Some(Wasm), WASM-EXECUTION: Compiled, CHAIN: Some("agung"), DB CACHE: 1024

// Executed Command:
// ./target/release/peaq-node
// benchmark
// pallet
// --chain
// agung
// --execution
// wasm
// --wasm-execution
// compiled
// --pallet
// pallet_block_reward
// --extrinsic
// *
// --steps
// 20
// --repeat
// 10
// --output
// weights.dev.rs

#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]

use frame_support::{traits::Get, weights::{Weight, constants::RocksDbWeight}};
use sp_std::marker::PhantomData;

/// Weight functions needed for pallet-reward-distribution.
pub trait WeightInfo {
    fn set_configuration() -> Weight;
	fn set_block_issue_reward() -> Weight;
	fn set_max_currency_supply() -> Weight;
}

/// Weight functions for `pallet_block_reward`.
pub struct SubstrateWeight<T>(PhantomData<T>);
impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
	// Storage: BlockReward RewardDistributionConfigStorage (r:0 w:1)
	fn set_configuration() -> Weight {
		Weight::from_parts(11_642_000, 0)
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
	// Storage: BlockReward BlockIssueReward (r:0 w:1)
	fn set_block_issue_reward() -> Weight {
		Weight::from_parts(10_850_000, 0)
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
	// Storage: BlockReward HardCap (r:0 w:1)
	fn set_max_currency_supply() -> Weight {
		Weight::from_parts(10_811_000, 0)
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
}
