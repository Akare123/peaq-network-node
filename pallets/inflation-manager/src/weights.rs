
//! Autogenerated weights for `inflation_manager`
//!
//! THIS FILE WAS AUTO-GENERATED USING THE SUBSTRATE BENCHMARK CLI VERSION 4.0.0-dev
//! DATE: 2024-05-09, STEPS: `50`, REPEAT: `20`, LOW RANGE: `[]`, HIGH RANGE: `[]`
//! WORST CASE MAP SIZE: `1000000`
//! HOSTNAME: `jaypan-peaq`, CPU: `AMD Ryzen 5 5600H with Radeon Graphics`
//! EXECUTION: Some(Native), WASM-EXECUTION: Compiled, CHAIN: Some("dev-local"), DB CACHE: 1024

// Executed Command:
// ./target/release/peaq-node
// benchmark
// pallet
// --chain=dev-local
// --execution=native
// --wasm-execution=compiled
// --pallet=inflation_manager
// --extrinsic=*
// --steps=50
// --repeat=20
// --output=weight.rs

#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]
#![allow(missing_docs)]

use frame_support::{traits::Get, weights::Weight};
use core::marker::PhantomData;

/// Weight functions for `inflation_manager`.
pub struct WeightInfo<T>(PhantomData<T>);
impl<T: frame_system::Config> crate::WeightInfo for WeightInfo<T> {
	/// Storage: System Account (r:2 w:2)
	/// Proof: System Account (max_values: None, max_size: Some(128), added: 2603, mode: MaxEncodedLen)
	/// Storage: AddressUnification EvmAddresses (r:1 w:0)
	/// Proof: AddressUnification EvmAddresses (max_values: None, max_size: Some(60), added: 2535, mode: MaxEncodedLen)
	fn transfer_all_pot() -> Weight {
		// Proof Size summary in bytes:
		//  Measured:  `272`
		//  Estimated: `6196`
		// Minimum execution time: 220_389_000 picoseconds.
		Weight::from_parts(222_362_000, 0)
			.saturating_add(Weight::from_parts(0, 6196))
			.saturating_add(T::DbWeight::get().reads(3))
			.saturating_add(T::DbWeight::get().writes(2))
	}
}
