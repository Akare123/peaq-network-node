// This file is part of Acala.

// Copyright (C) 2020-2022 Acala Foundation.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

//! Autogenerated weights for module_asset_registry
//!
//! THIS FILE WAS AUTO-GENERATED USING THE SUBSTRATE BENCHMARK CLI VERSION 4.0.0-dev
//! DATE: 2022-03-24, STEPS: `50`, REPEAT: 20, LOW RANGE: `[]`, HIGH RANGE: `[]`
//! EXECUTION: Some(Wasm), WASM-EXECUTION: Compiled, CHAIN: Some("dev"), DB CACHE: 1024

// Executed Command:
// target/release/acala
// benchmark
// --chain=dev
// --steps=50
// --repeat=20
// --pallet=module_asset_registry
// --extrinsic=*
// --execution=wasm
// --wasm-execution=compiled
// --heap-pages=4096
// --output=./modules/asset-registry/src/weights.rs
// --template=./templates/module-weight-template.hbs

#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]
#![allow(clippy::unnecessary_cast)]

use frame_support::{traits::Get, weights::{Weight, constants::RocksDbWeight}};
use sp_std::marker::PhantomData;

/// Weight functions needed for module_asset_registry.
pub trait WeightInfo {
	fn register_foreign_asset() -> Weight;
	fn update_foreign_asset() -> Weight;
	fn register_stable_asset() -> Weight;
	fn update_stable_asset() -> Weight;
	fn register_erc20_asset() -> Weight;
	fn update_erc20_asset() -> Weight;
	fn register_native_asset() -> Weight;
	fn update_native_asset() -> Weight;
}

/// Weights for module_asset_registry using the Acala node and recommended hardware.
pub struct SubstrateWeight<T>(PhantomData<T>);
impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
	// Storage: AssetRegistry NextForeignAssetId (r:1 w:1)
	// Storage: AssetRegistry LocationToCurrencyIds (r:1 w:1)
	// Storage: AssetRegistry ForeignAssetLocations (r:1 w:1)
	// Storage: AssetRegistry AssetMetadatas (r:1 w:1)
	fn register_foreign_asset() -> Weight {
		Weight::from_ref_time(21_475_000)
			.saturating_add(T::DbWeight::get().reads(4 as u64))
			.saturating_add(T::DbWeight::get().writes(4 as u64))
	}
	// Storage: AssetRegistry ForeignAssetLocations (r:1 w:1)
	// Storage: AssetRegistry AssetMetadatas (r:1 w:1)
	fn update_foreign_asset() -> Weight {
		Weight::from_ref_time(19_334_000)
			.saturating_add(T::DbWeight::get().reads(2 as u64))
			.saturating_add(T::DbWeight::get().writes(2 as u64))
	}
	// Storage: AssetRegistry NextStableAssetId (r:1 w:1)
	// Storage: AssetRegistry AssetMetadatas (r:1 w:1)
	fn register_stable_asset() -> Weight {
		Weight::from_ref_time(15_830_000)
			.saturating_add(T::DbWeight::get().reads(2 as u64))
			.saturating_add(T::DbWeight::get().writes(2 as u64))
	}
	// Storage: AssetRegistry AssetMetadatas (r:1 w:1)
	fn update_stable_asset() -> Weight {
		Weight::from_ref_time(14_342_000)
			.saturating_add(T::DbWeight::get().reads(1 as u64))
			.saturating_add(T::DbWeight::get().writes(1 as u64))
	}
	// Storage: EVM Accounts (r:2 w:0)
	// Storage: EVM Codes (r:1 w:0)
	// Storage: EVM AccountStorages (r:5 w:0)
	// Storage: AssetRegistry AssetMetadatas (r:1 w:1)
	// Storage: AssetRegistry Erc20IdToAddress (r:1 w:1)
	fn register_erc20_asset() -> Weight {
		Weight::from_ref_time(187_828_000)
			.saturating_add(T::DbWeight::get().reads(10 as u64))
			.saturating_add(T::DbWeight::get().writes(2 as u64))
	}
	// Storage: AssetRegistry AssetMetadatas (r:1 w:1)
	fn update_erc20_asset() -> Weight {
		Weight::from_ref_time(19_773_000)
			.saturating_add(T::DbWeight::get().reads(1 as u64))
			.saturating_add(T::DbWeight::get().writes(1 as u64))
	}
	// Storage: AssetRegistry AssetMetadatas (r:1 w:1)
	fn register_native_asset() -> Weight {
		Weight::from_ref_time(13_140_000)
			.saturating_add(T::DbWeight::get().reads(1 as u64))
			.saturating_add(T::DbWeight::get().writes(1 as u64))
	}
	// Storage: AssetRegistry AssetMetadatas (r:1 w:1)
	fn update_native_asset() -> Weight {
		Weight::from_ref_time(13_815_000)
			.saturating_add(T::DbWeight::get().reads(1 as u64))
			.saturating_add(T::DbWeight::get().writes(1 as u64))
	}
}

// For backwards compatibility and tests
impl WeightInfo for () {
	fn register_foreign_asset() -> Weight {
		Weight::from_ref_time(21_475_000)
			.saturating_add(RocksDbWeight::get().reads(4 as u64))
			.saturating_add(RocksDbWeight::get().writes(4 as u64))
	}
	fn update_foreign_asset() -> Weight {
		Weight::from_ref_time(19_334_000)
			.saturating_add(RocksDbWeight::get().reads(2 as u64))
			.saturating_add(RocksDbWeight::get().writes(2 as u64))
	}
	fn register_stable_asset() -> Weight {
		Weight::from_ref_time(15_830_000)
			.saturating_add(RocksDbWeight::get().reads(2 as u64))
			.saturating_add(RocksDbWeight::get().writes(2 as u64))
	}
	fn update_stable_asset() -> Weight {
		Weight::from_ref_time(14_342_000)
			.saturating_add(RocksDbWeight::get().reads(1 as u64))
			.saturating_add(RocksDbWeight::get().writes(1 as u64))
	}
	fn register_erc20_asset() -> Weight {
		Weight::from_ref_time(187_828_000)
			.saturating_add(RocksDbWeight::get().reads(10 as u64))
			.saturating_add(RocksDbWeight::get().writes(2 as u64))
	}
	fn update_erc20_asset() -> Weight {
		Weight::from_ref_time(19_773_000)
			.saturating_add(RocksDbWeight::get().reads(1 as u64))
			.saturating_add(RocksDbWeight::get().writes(1 as u64))
	}
	fn register_native_asset() -> Weight {
		Weight::from_ref_time(13_140_000)
			.saturating_add(RocksDbWeight::get().reads(1 as u64))
			.saturating_add(RocksDbWeight::get().writes(1 as u64))
	}
	fn update_native_asset() -> Weight {
		Weight::from_ref_time(13_815_000)
			.saturating_add(RocksDbWeight::get().reads(1 as u64))
			.saturating_add(RocksDbWeight::get().writes(1 as u64))
	}
}
