// KILT Blockchain – https://botlabs.org
// Copyright (C) 2019-2023 BOTLabs GmbH

// The KILT Blockchain is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// The KILT Blockchain is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

// If you feel like getting in touch with us, you can do so at info@botlabs.org

//! # Parachain Staking
//!
//! A simple staking pallet providing means of selecting a set of collators to
//! become block authors based on their total backed stake. The main difference
//! between this pallet and `frame/pallet-staking` is that this pallet uses
//! direct delegation. Delegators choose exactly who they delegate and with what
//! stake. This is different from `frame/pallet-staking` where you approval vote
//! and then run Phragmen. Moreover, this pallet rewards a collator and their
//! delegators immediately when authoring a block. Rewards are calculated
//! separately between collators and delegators.
//!
//! To join the set of candidates, an account must call `join_candidates` with
//! `MinCollatorCandidateStake` <= stake <= `MaxCollatorCandidateStake`.
//!
//! To leave the set of candidates, the collator calls `leave_candidates`. If
//! the call succeeds, the collator is removed from the pool of candidates so
//! they cannot be selected for future collator sets, but they are not unstaking
//! until executing the exit request by calling the extrinsic
//! `execute_leave_candidates` at least `ExitQueueDelay` rounds later. After
//! doing so, the collator candidate as well as their delegators are unstaked.
//! Both parties then have to wait another `StakeDuration` more blocks to be
//! able to unlock their stake.
//!
//! Candidates which requested to leave can still be in the set of authors for
//! the next round due to the design of the session pallet which at the start of
//! session s(i) chooses a set for the next session s(i+1). Thus, candidates
//! have to keep collating at least until the end of the next session (= round).
//! We extend this by delaying their execute by at least `ExitQueueDelay` many
//! sessions.
//!
//! To join the set of delegators, an account must call `join_delegators` with
//! stake >= `MinDelegatorStake`. There are also runtime methods for delegating
//! additional collators and revoking delegations.
//!
//!
//! - [`Config`]
//! - [`Call`]
//! - [`Pallet`]
//!
//! ## Overview
//!
//! The KILT parachain staking pallet provides functions for:
//! - Joining the set of collator candidates of which the best `MaxSelectedCandidates` are chosen to
//!   become active collators for the next session. That makes the set of active collators the set
//!   of block authors by handing it over to the session and the authority pallet.
//! - Delegating to a collator candidate by staking for them.
//! - Increasing and reducing your stake as a collator or delegator.
//! - Revoking your delegation entirely.
//! - Requesting to leave the set of collator candidates.
//! - Withdrawing your unstaked balance after waiting for a certain number of blocks.
//!
//! ### Terminology
//!
//! - **Candidate:** A user which locks up tokens to be included into the set of authorities which
//!   author blocks and receive rewards for doing so.
//!
//! - **Collator:** A candidate that was chosen to collate this round.
//!
//! - **Delegator:** A user which locks up tokens for collators they trust. When their collator
//!   authors a block, the corresponding delegators also receive rewards.
//!
//! - **Total Stake:** A collator’s own stake + the sum of delegated stake to this collator.
//!
//! - **Total collator stake:** The sum of tokens locked for staking from all collator candidates.
//!
//! - **Total delegator stake:** The sum of tokens locked for staking from all delegators.
//!
//! - **To Stake:** Lock tokens for staking.
//!
//! - **To Unstake:** Unlock tokens from staking.
//!
//! - **Round (= Session):** A fixed number of blocks in which the set of collators does not change.
//!   We set the length of a session to the length of a staking round, thus both words are
//!   interchangeable in the context of this pallet.
//!
//! - **Lock:** A freeze on a specified amount of an account's free balance until a specified block
//!   number. Multiple locks always operate over the same funds, so they "overlay" rather than
//!   "stack"
//!
//! ## Genesis config
//!
//! The ParachainStaking pallet depends on the [`GenesisConfig`].
//!
//! ## Assumptions+
//!
//! - At the start of session s(i), the set of session ids for session s(i+1) are chosen. These
//!   equal the set of selected candidates. Thus, we cannot allow collators to leave at least until
//!   the start of session s(i+2).

#![cfg_attr(not(feature = "std"), no_std)]
#![allow(clippy::unused_unit)]

pub use crate::{default_weights::WeightInfo, pallet::*};

#[cfg(feature = "runtime-benchmarks")]
pub mod benchmarking;
pub mod default_weights;

pub mod migration;
#[cfg(test)]
pub(crate) mod mock;
#[cfg(test)]
pub(crate) mod tests;

pub mod api;
mod reward_rate;
mod set;
mod types;

use frame_support::pallet;

#[macro_export]
macro_rules! log {
	($level:tt, $patter:expr $(, $values:expr)* $(,)?) => {
		log::$level!(
			target: "runtime::parachain-staking",
			concat!("[{:?}] 💸 ", $patter), <frame_system::Pallet<T>>::block_number() $(, $values)*
		)
	};
}

#[pallet]
pub mod pallet {
	use super::*;
	pub use crate::reward_rate::RewardRateInfo;

	use frame_support::{
		assert_ok,
		pallet_prelude::*,
		storage::bounded_btree_map::BoundedBTreeMap,
		traits::{
			Currency, EstimateNextSessionRotation, Get, Imbalance, LockIdentifier,
			LockableCurrency, OnUnbalanced, ReservableCurrency, StorageVersion, WithdrawReasons,
		},
		weights::Weight,
		BoundedVec, PalletId,
	};
	use frame_system::pallet_prelude::*;
	use pallet_balances::{BalanceLock, Locks};
	use pallet_block_reward::averaging::ProvidesAverageFor;
	use pallet_session::ShouldEndSession;
	use scale_info::TypeInfo;
	use sp_runtime::{
		traits::{
			AccountIdConversion, Convert, One, SaturatedConversion, Saturating, StaticLookup, Zero,
		},
		Permill, Perquintill,
	};
	use sp_staking::SessionIndex;
	use sp_std::prelude::*;

	use crate::{set::OrderedSet, types::*};
	use sp_std::{convert::TryInto, fmt::Debug};

	/// Kilt-specific lock for staking rewards.
	pub(crate) const STAKING_ID: LockIdentifier = *b"kiltpstk";

	/// The current storage version.
	const STORAGE_VERSION: StorageVersion = StorageVersion::new(8);

	/// Pallet for parachain staking.
	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	#[pallet::storage_version(STORAGE_VERSION)]
	pub struct Pallet<T>(PhantomData<T>);

	/// Configuration trait of this pallet.
	#[pallet::config]
	pub trait Config:
		frame_system::Config + pallet_balances::Config + pallet_session::Config
	{
		/// Overarching event type
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;
		// FIXME: Remove Currency and CurrencyBalance types. Problem: Need to restrict
		// pallet_balances::Config::Balance with From<u64> for usage with Perquintill
		// multiplication
		/// The currency type
		/// Note: Declaration of Balance taken from pallet_gilt
		type Currency: Currency<Self::AccountId, Balance = Self::CurrencyBalance>
			+ ReservableCurrency<Self::AccountId, Balance = Self::CurrencyBalance>
			+ LockableCurrency<Self::AccountId, Balance = Self::CurrencyBalance>
			+ Eq;

		/// Just the `Currency::Balance` type; we have this item to allow us to
		/// constrain it to `From<u64>`.
		/// Note: Definition taken from pallet_gilt
		type CurrencyBalance: sp_runtime::traits::AtLeast32BitUnsigned
			+ codec::FullCodec
			+ Copy
			+ MaybeSerializeDeserialize
			+ sp_std::fmt::Debug
			+ Default
			+ From<u64>
			+ From<u128>
			+ Into<<Self as pallet_balances::Config>::Balance>
			+ From<<Self as pallet_balances::Config>::Balance>
			+ From<Self::BlockNumber>
			+ TypeInfo
			+ MaxEncodedLen;
		
		/// The provider for the average-block-reward.
		// type AvgBlockRewardProvider: ProvidesAverageFor<Recipient: Self::AvgRecipientSelector>;
		type AvgBlockRewardProvider: ProvidesAverageFor<Self::CurrencyBalance, Self::AvgRecipientSelector>;

		/// The recipient-selector-datatype for ProvidesAverageFor.
		type AvgRecipientSelector: Parameter;

		/// Const which actually selects this pallet as recipient.
		#[pallet::constant]
		type AvgBlockRewardRecipient: Get<Self::AvgRecipientSelector>;

		/// Minimum number of blocks validation rounds can last.
		#[pallet::constant]
		type MinBlocksPerRound: Get<Self::BlockNumber>;

		/// Default number of blocks validation rounds last, as set in the
		/// genesis configuration.
		#[pallet::constant]
		type DefaultBlocksPerRound: Get<Self::BlockNumber>;
		/// Number of blocks for which unstaked balance will still be locked
		/// before it can be unlocked by actively calling the extrinsic
		/// `unlock_unstaked`.
		#[pallet::constant]
		type StakeDuration: Get<Self::BlockNumber>;
		/// Number of rounds a collator has to stay active after submitting a
		/// request to leave the set of collator candidates.
		#[pallet::constant]
		type ExitQueueDelay: Get<u32>;

		/// Minimum number of collators selected from the set of candidates at
		/// every validation round.
		#[pallet::constant]
		type MinCollators: Get<u32>;

		/// Minimum number of collators which cannot leave the network if there
		/// are no others.
		#[pallet::constant]
		type MinRequiredCollators: Get<u32>;

		/// Maximum number of delegations which can be made within the same
		/// round.
		///
		/// NOTE: To prevent re-delegation-reward attacks, we should keep this
		/// to be one.
		#[pallet::constant]
		type MaxDelegationsPerRound: Get<u32>;

		/// Maximum number of delegators a single collator can have.
		#[pallet::constant]
		type MaxDelegatorsPerCollator: Get<u32> + Debug + PartialEq;

		/// Maximum size of the top candidates set.
		#[pallet::constant]
		type MaxTopCandidates: Get<u32> + Debug + PartialEq;

		/// Minimum stake required for any account to be elected as validator
		/// for a round.
		#[pallet::constant]
		type MinCollatorStake: Get<BalanceOf<Self>>;

		/// Minimum stake required for any account to be added to the set of
		/// candidates.
		#[pallet::constant]
		type MinCollatorCandidateStake: Get<BalanceOf<Self>>;

		/// Minimum stake required for any account to become a delegator.
		#[pallet::constant]
		type MinDelegatorStake: Get<BalanceOf<Self>>;

		/// Max number of concurrent active unstaking requests before
		/// unlocking.
		///
		/// NOTE: To protect against irremovability of a candidate or delegator,
		/// we only allow for MaxUnstakeRequests - 1 many manual unstake
		/// requests. The last one serves as a placeholder for the cases of
		/// calling either `kick_delegator`, force_remove_candidate` or
		/// `execute_leave_candidates`. Otherwise, a user could max out their
		/// unstake requests and prevent themselves from being kicked from the
		/// set of candidates/delegators until they unlock their funds.
		#[pallet::constant]
		type MaxUnstakeRequests: Get<u32>;

		/// Account Identifier from which the internal Pot is generated.
		#[pallet::constant]
		type PotId: Get<PalletId>;

		/// Weight information for extrinsics in this pallet.
		type WeightInfo: WeightInfo;
	}

	#[pallet::error]
	pub enum Error<T> {
		/// The account is not part of the delegators set.
		DelegatorNotFound,
		/// The account is not part of the collator candidates set.
		CandidateNotFound,
		/// The account is already part of the delegators set.
		DelegatorExists,
		/// The account is already part of the collator candidates set.
		CandidateExists,
		/// The account tried to stake more or less with amount zero.
		ValStakeZero,
		/// The account has not staked enough funds to be added to the collator
		/// candidates set.
		ValStakeBelowMin,
		/// The account has already staked the maximum amount of funds possible.
		ValStakeAboveMax,
		/// The account has not staked enough funds to delegate a collator
		/// candidate.
		DelegationBelowMin,
		/// The collator candidate has already trigger the process to leave the
		/// set of collator candidates.
		AlreadyLeaving,
		/// The collator candidate wanted to execute the exit but has not
		/// requested to leave before by calling `init_leave_candidates`.
		NotLeaving,
		/// The collator tried to leave before waiting at least for
		/// `ExitQueueDelay` many rounds.
		CannotLeaveYet,
		/// The account has a full list of unstaking requests and needs to
		/// unlock at least one of these before being able to join (again).
		/// NOTE: Can only happen if the account was a candidate or
		/// delegator before and either got kicked or exited voluntarily.
		CannotJoinBeforeUnlocking,
		/// The account is already delegating the collator candidate.
		AlreadyDelegating,
		/// The delegator has exceeded the number of delegations per round which
		/// is equal to MaxDelegatorsPerCollator.
		///
		/// This protects against attacks in which a delegator can re-delegate
		/// from a collator who has already authored a block, to another one
		/// which has not in this round.
		DelegationsPerRoundExceeded,
		/// The collator candidate has already reached the maximum number of
		/// delegators.
		///
		/// This error is generated in case a new delegation request does not
		/// stake enough funds to replace some other existing delegation.
		TooManyDelegators,
		/// The set of collator candidates would fall below the required minimum
		/// if the collator left.
		TooFewCollatorCandidates,
		/// The collator candidate is in the process of leaving the set of
		/// candidates and cannot perform any other actions in the meantime.
		CannotStakeIfLeaving,
		/// The collator candidate is in the process of leaving the set of
		/// candidates and thus cannot be delegated to.
		CannotDelegateIfLeaving,
		/// The given delegation does not exist in the set of delegations.
		DelegationNotFound,
		/// The collator delegate or the delegator is trying to un-stake more
		/// funds that are currently staked.
		Underflow,
		/// The number of selected candidates per staking round is
		/// above the maximum value allowed.
		CannotSetAboveMax,
		/// The number of selected candidates per staking round is
		/// below the minimum value allowed.
		CannotSetBelowMin,
		/// An invalid reward rate configuration is trying to be set.
		InvalidSchedule,
		/// The staking reward being unlocked does not exist.
		/// Max unlocking requests reached.
		NoMoreUnstaking,
		/// Provided staked value is zero. Should never be thrown.
		StakeNotFound,
		/// Cannot unlock when Unstaked is empty.
		UnstakingIsEmpty,
		/// Cannot claim rewards if empty.
		RewardsNotFound,
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(crate) fn deposit_event)]
	pub enum Event<T: Config> {
		/// A new staking round has started.
		/// \[block number, round number\]
		NewRound(T::BlockNumber, SessionIndex),
		/// A new account has joined the set of top candidates.
		/// \[account\]
		EnteredTopCandidates(T::AccountId),
		/// An account was removed from the set of top candidates.
		/// \[account\]
		LeftTopCandidates(T::AccountId),
		/// A new account has joined the set of collator candidates.
		/// \[account, amount staked by the new candidate\]
		JoinedCollatorCandidates(T::AccountId, BalanceOf<T>),
		/// A collator candidate has increased the amount of funds at stake.
		/// \[collator's account, previous stake, new stake\]
		CollatorStakedMore(T::AccountId, BalanceOf<T>, BalanceOf<T>),
		/// A collator candidate has decreased the amount of funds at stake.
		/// \[collator's account, previous stake, new stake\]
		CollatorStakedLess(T::AccountId, BalanceOf<T>, BalanceOf<T>),
		/// A collator candidate has started the process to leave the set of
		/// candidates. \[round number, collator's account, round number when
		/// the collator will be effectively removed from the set of
		/// candidates\]
		CollatorScheduledExit(SessionIndex, T::AccountId, SessionIndex),
		/// A collator candidate has canceled the process to leave the set of
		/// candidates and was added back to the candidate pool. \[collator's
		/// account\]
		CollatorCanceledExit(T::AccountId),
		/// An account has left the set of collator candidates.
		/// \[account, amount of funds un-staked\]
		CandidateLeft(T::AccountId, BalanceOf<T>),
		/// An account was forcedly removed from the  set of collator
		/// candidates. \[account, amount of funds un-staked\]
		CollatorRemoved(T::AccountId, BalanceOf<T>),
		/// The maximum candidate stake has been changed.
		/// \[new max amount\]
		MaxCandidateStakeChanged(BalanceOf<T>),
		/// A delegator has increased the amount of funds at stake for a
		/// collator. \[delegator's account, collator's account, previous
		/// delegation stake, new delegation stake\]
		DelegatorStakedMore(T::AccountId, T::AccountId, BalanceOf<T>, BalanceOf<T>),
		/// A delegator has decreased the amount of funds at stake for a
		/// collator. \[delegator's account, collator's account, previous
		/// delegation stake, new delegation stake\]
		DelegatorStakedLess(T::AccountId, T::AccountId, BalanceOf<T>, BalanceOf<T>),
		/// An account has left the set of delegators.
		/// \[account, amount of funds un-staked\]
		DelegatorLeft(T::AccountId, BalanceOf<T>),
		/// An account has delegated a new collator candidate.
		/// \[account, amount of funds staked, total amount of delegators' funds
		/// staked for the collator candidate\]
		Delegation(T::AccountId, BalanceOf<T>, T::AccountId, BalanceOf<T>),
		/// A new delegation has replaced an existing one in the set of ongoing
		/// delegations for a collator candidate. \[new delegator's account,
		/// amount of funds staked in the new delegation, replaced delegator's
		/// account, amount of funds staked in the replace delegation, collator
		/// candidate's account, new total amount of delegators' funds staked
		/// for the collator candidate\]
		DelegationReplaced(
			T::AccountId,
			BalanceOf<T>,
			T::AccountId,
			BalanceOf<T>,
			T::AccountId,
			BalanceOf<T>,
		),
		/// An account has stopped delegating a collator candidate.
		/// \[account, collator candidate's account, old amount of delegators'
		/// funds staked, new amount of delegators' funds staked\]
		DelegatorLeftCollator(T::AccountId, T::AccountId, BalanceOf<T>, BalanceOf<T>),
		/// A collator or a delegator has received a reward.
		/// \[account, amount of reward\]
		Rewarded(T::AccountId, BalanceOf<T>),
		/// Reware rate configuration for future validation rounds has changed.
		/// \[collator's reward rate,delegator's reward rate\]
		RoundRewardRateSet(Perquintill, Perquintill),
		/// The maximum number of collator candidates selected in future
		/// validation rounds has changed. \[old value, new value\]
		MaxSelectedCandidatesSet(u32, u32),
		/// The length in blocks for future validation rounds has changed.
		/// \[round number, first block in the current round, old value, new
		/// value\]
		BlocksPerRoundSet(SessionIndex, T::BlockNumber, T::BlockNumber, T::BlockNumber),
		/// If sudo-user resets the average reward register.
		AverageRewardReset(BalanceOf<T>),
	}

	/// The maximum number of collator candidates selected at each round.
	#[pallet::storage]
	#[pallet::getter(fn max_selected_candidates)]
	pub(crate) type MaxSelectedCandidates<T: Config> = StorageValue<_, u32, ValueQuery>;

	/// Current round number and next round scheduled transition.
	#[pallet::storage]
	#[pallet::getter(fn round)]
	pub(crate) type Round<T: Config> = StorageValue<_, RoundInfo<T::BlockNumber>, ValueQuery>;

	/// Delegation information for the latest session in which a delegator
	/// delegated.
	///
	/// It maps from an account to the number of delegations in the last
	/// session in which they (re-)delegated.
	#[pallet::storage]
	#[pallet::getter(fn last_delegation)]
	pub(crate) type LastDelegation<T: Config> =
		StorageMap<_, Twox64Concat, T::AccountId, DelegationCounter, ValueQuery>;

	/// Delegation staking information.
	///
	/// It maps from an account to its delegation details.
	#[pallet::storage]
	#[pallet::getter(fn delegator_state)]
	pub(crate) type DelegatorState<T: Config> = StorageMap<
		_,
		Twox64Concat,
		T::AccountId,
		Delegator<T::AccountId, BalanceOf<T>>,
		OptionQuery,
	>;

	/// The staking information for a candidate.
	///
	/// It maps from an account to its information.
	/// Moreover, it counts the number of candidates.
	#[pallet::storage]
	#[pallet::getter(fn candidate_pool)]
	pub(crate) type CandidatePool<T: Config> = CountedStorageMap<
		_,
		Twox64Concat,
		T::AccountId,
		Candidate<T::AccountId, BalanceOf<T>, T::MaxDelegatorsPerCollator>,
		OptionQuery,
	>;

	/// Total funds locked to back the currently selected collators.
	/// The sum of all collator and their delegator stakes.
	///
	/// Note: There are more funds locked by this pallet, since the backing for
	/// non collating candidates is not included in [TotalCollatorStake].
	#[pallet::storage]
	#[pallet::getter(fn total_collator_stake)]
	pub(crate) type TotalCollatorStake<T: Config> =
		StorageValue<_, TotalStake<BalanceOf<T>>, ValueQuery>;

	/// The collator candidates with the highest amount of stake.
	///
	/// Each time the stake of a collator is increased, it is checked whether
	/// this pushes another candidate out of the list. When the stake is
	/// reduced however, it is not checked if another candidate has more stake,
	/// since this would require iterating over the entire [CandidatePool].
	///
	/// There must always be more candidates than [MaxSelectedCandidates] so
	/// that a collator can drop out of the collator set by reducing their
	/// stake.
	#[pallet::storage]
	#[pallet::getter(fn top_candidates)]
	pub(crate) type TopCandidates<T: Config> = StorageValue<
		_,
		OrderedSet<Stake<T::AccountId, BalanceOf<T>>, T::MaxTopCandidates>,
		ValueQuery,
	>;

	/// Reward rate configuration.
	#[pallet::storage]
	#[pallet::getter(fn reward_rate_config)]
	pub(crate) type RewardRateConfig<T: Config> = StorageValue<_, RewardRateInfo, ValueQuery>;

	/// The funds waiting to be unstaked.
	///
	/// It maps from accounts to all the funds addressed to them in the future
	/// blocks.
	#[pallet::storage]
	#[pallet::getter(fn unstaking)]
	pub(crate) type Unstaking<T: Config> = StorageMap<
		_,
		Twox64Concat,
		T::AccountId,
		BoundedBTreeMap<T::BlockNumber, BalanceOf<T>, T::MaxUnstakeRequests>,
		ValueQuery,
	>;

	/// The maximum amount a collator candidate can stake.
	#[pallet::storage]
	#[pallet::getter(fn max_candidate_stake)]
	pub(crate) type MaxCollatorCandidateStake<T: Config> =
		StorageValue<_, BalanceOf<T>, ValueQuery>;

	/// The number of authored blocks for collators. It is updated via the
	/// `note_author` hook when authoring a block .
	#[pallet::storage]
	#[pallet::getter(fn blocks_authored)]
	pub(crate) type BlocksAuthored<T: Config> =
		StorageMap<_, Twox64Concat, T::AccountId, T::BlockNumber, ValueQuery>;

	/// The number of blocks for which rewards have been claimed by an address.
	///
	/// For collators, this can be at most BlocksAuthored. It is updated when
	/// incrementing collator rewards, either when calling
	/// `inc_collator_rewards` or updating the `InflationInfo`.
	///
	/// For delegators, this can be at most BlocksAuthored of the collator.It is
	/// updated when incrementing delegator rewards, either when calling
	/// `inc_delegator_rewards` or updating the `InflationInfo`.
	#[pallet::storage]
	#[pallet::getter(fn blocks_rewarded)]
	pub(crate) type BlocksRewarded<T: Config> =
		StorageMap<_, Twox64Concat, T::AccountId, T::BlockNumber, ValueQuery>;

	/// The accumulated rewards for collator candidates and delegators.
	///
	/// It maps from accounts to their total rewards since the last payout.
	#[pallet::storage]
	#[pallet::getter(fn rewards)]
	pub(crate) type Rewards<T: Config> =
		StorageMap<_, Twox64Concat, T::AccountId, BalanceOf<T>, ValueQuery>;

	// /// The average total reward, given to this pallet. Because currently, there
	// /// is no dynamic issue of tokens, we need an average value as reference for
	// /// payouts. The storage is a tuple with (AverageBlockReward, Accumulator).
	// /// The first value is the value to be used in calculations. The second one
	// /// is about calculating the first value properly (solves the problem with
	// /// multiple incoming block-rewards).
	// #[pallet::storage]
	// #[pallet::getter(fn average_block_reward)]
	// pub(crate) type AverageBlockReward<T: Config> =
	// 	StorageValue<_, AvgBlockRewardCtrl<BalanceOf<T>>, ValueQuery>;

	pub type GenesisStaker<T> = Vec<(
		<T as frame_system::Config>::AccountId,
		Option<<T as frame_system::Config>::AccountId>,
		BalanceOf<T>,
	)>;

	#[pallet::storage]
	#[pallet::getter(fn new_round_forced)]
	pub(crate) type ForceNewRound<T: Config> = StorageValue<_, bool, ValueQuery>;

	#[pallet::genesis_config]
	pub struct GenesisConfig<T: Config> {
		pub stakers: GenesisStaker<T>,
		pub reward_rate_config: RewardRateInfo,
		pub max_candidate_stake: BalanceOf<T>,
	}

	#[cfg(feature = "std")]
	impl<T: Config> Default for GenesisConfig<T> {
		fn default() -> Self {
			Self {
				stakers: Default::default(),
				reward_rate_config: Default::default(),
				max_candidate_stake: Default::default(),
			}
		}
	}

	#[pallet::genesis_build]
	impl<T: Config> GenesisBuild<T> for GenesisConfig<T> {
		fn build(&self) {
			assert!(self.reward_rate_config.is_valid(), "Invalid reward_rate configuration");

			RewardRateConfig::<T>::put(self.reward_rate_config.clone());
			MaxCollatorCandidateStake::<T>::put(self.max_candidate_stake);

			// Setup delegate & collators
			for &(ref actor, ref opt_val, balance) in &self.stakers {
				assert!(
					T::Currency::free_balance(actor) >= balance,
					"Account does not have enough balance to stake."
				);
				if let Some(delegated_val) = opt_val {
					assert_ok!(Pallet::<T>::join_delegators(
						T::Origin::from(Some(actor.clone()).into()),
						T::Lookup::unlookup(delegated_val.clone()),
						balance,
					));
				} else {
					assert_ok!(<Pallet<T>>::join_candidates(
						T::Origin::from(Some(actor.clone()).into()),
						balance
					));
				}
			}
			// Set total selected candidates to minimum config
			MaxSelectedCandidates::<T>::put(T::MinCollators::get());

			Pallet::<T>::update_total_stake();

			// Start Round 0 at Block 0
			let round: RoundInfo<T::BlockNumber> =
				RoundInfo::new(0u32, 0u32.into(), T::DefaultBlocksPerRound::get());
			Round::<T>::put(round);
		}
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_initialize(now: T::BlockNumber) -> Weight {
			let mut post_weight = <T as Config>::WeightInfo::on_initialize_no_action();

			// check for round update
			let mut round = <Round<T>>::get();
			if round.should_update(now) {
				// mutate round
				round.update(now);

				// start next round
				Round::<T>::put(round);

				Self::deposit_event(Event::NewRound(round.first, round.current));
				post_weight = <T as Config>::WeightInfo::on_initialize_round_update();
			}
			post_weight
		}
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Forces the start of the new round in the next block.
		///
		/// The new round will be enforced via <T as
		/// ShouldEndSession<_>>::should_end_session.
		///
		/// The dispatch origin must be Root.
		#[pallet::weight(<T as pallet::Config>::WeightInfo::set_reward_rate())]
		pub fn force_new_round(origin: OriginFor<T>) -> DispatchResult {
			ensure_root(origin)?;

			// set force_new_round handle which, at the start of the next block, will
			// trigger `should_end_session` in `Session::on_initialize` and update the
			// current round
			ForceNewRound::<T>::put(true);

			Ok(())
		}

		/// Set the reward_rate rate.
		///
		/// The estimated average block time is twelve seconds.
		///
		/// The dispatch origin must be Root.
		///
		/// Emits `RoundRewardRateSet`.
		#[pallet::weight(<T as pallet::Config>::WeightInfo::set_reward_rate())]
		pub fn set_reward_rate(
			origin: OriginFor<T>,
			collator_rate: Perquintill,
			delegator_rate: Perquintill,
		) -> DispatchResult {
			ensure_root(origin)?;

			let reward_rate = RewardRateInfo::new(collator_rate, delegator_rate);

			ensure!(reward_rate.is_valid(), Error::<T>::InvalidSchedule);
			Self::deposit_event(Event::RoundRewardRateSet(
				reward_rate.collator_rate,
				reward_rate.delegator_rate,
			));
			RewardRateConfig::<T>::put(reward_rate);
			Ok(())
		}

		/// Set the maximum number of collator candidates that can be selected
		/// at the beginning of each validation round.
		///
		/// Changes are not applied until the start of the next round.
		///
		/// The new value must be higher than the minimum allowed as set in the
		/// pallet's configuration.
		///
		/// The dispatch origin must be Root.
		///
		/// Emits `MaxSelectedCandidatesSet`.
		#[pallet::weight(<T as pallet::Config>::WeightInfo::set_max_selected_candidates(
			*new,
			T::MaxDelegatorsPerCollator::get()
		))]
		pub fn set_max_selected_candidates(
			origin: OriginFor<T>,
			new: u32,
		) -> DispatchResultWithPostInfo {
			ensure_root(origin)?;
			ensure!(new >= T::MinCollators::get(), Error::<T>::CannotSetBelowMin);
			ensure!(new <= T::MaxTopCandidates::get(), Error::<T>::CannotSetAboveMax);
			let old = MaxSelectedCandidates::<T>::get();

			// *** No Fail beyond this point ***

			MaxSelectedCandidates::<T>::put(new);

			// Update total amount at stake for new top collators and their delegators
			let start = old.min(new);
			let end = old.max(new);

			// The slice [start, end] contains the added or removed collators. We sum up
			// their stake to adjust the total stake.
			let (diff_collation, diff_delegation, num_delegators) = TopCandidates::<T>::get()
				.into_iter()
				.skip(start.saturated_into())
				// SAFETY: we ensured that end > start further above.
				.take((end - start).saturated_into())
				.filter_map(|candidate| CandidatePool::<T>::get(&candidate.owner))
				.map(|state| {
					(
						state.stake,
						// SAFETY: the total is always more than the stake
						state.total - state.stake,
						state.delegators.len().saturated_into::<u32>(),
					)
				})
				.reduce(|a, b| (a.0.saturating_add(b.0), a.1.saturating_add(b.1), a.2.max(b.2)))
				.unwrap_or((BalanceOf::<T>::zero(), BalanceOf::<T>::zero(), 0u32));

			TotalCollatorStake::<T>::mutate(|total| {
				if new > old {
					total.collators = total.collators.saturating_add(diff_collation);
					total.delegators = total.delegators.saturating_add(diff_delegation);
				} else {
					total.collators = total.collators.saturating_sub(diff_collation);
					total.delegators = total.delegators.saturating_sub(diff_delegation);
				}
			});

			Self::deposit_event(Event::MaxSelectedCandidatesSet(old, new));

			Ok(Some(<T as pallet::Config>::WeightInfo::set_max_selected_candidates(
				// SAFETY: we ensured that end > start further above.
				end - start,
				num_delegators,
			))
			.into())
		}

		/// Set the number of blocks each validation round lasts.
		///
		/// If the new value is less than the length of the current round, the
		/// system will immediately move to the next round in the next block.
		///
		/// The new value must be higher than the minimum allowed as set in the
		/// pallet's configuration.
		///
		/// The dispatch origin must be Root.
		///
		/// Emits `BlocksPerRoundSet`.
		#[pallet::weight(<T as pallet::Config>::WeightInfo::set_blocks_per_round())]
		pub fn set_blocks_per_round(origin: OriginFor<T>, new: T::BlockNumber) -> DispatchResult {
			ensure_root(origin)?;
			ensure!(new >= T::MinBlocksPerRound::get(), Error::<T>::CannotSetBelowMin);

			let old_round = Round::<T>::get();

			// *** No Fail beyond this point ***

			Round::<T>::put(RoundInfo { length: new, ..old_round });

			Self::deposit_event(Event::BlocksPerRoundSet(
				old_round.current,
				old_round.first,
				old_round.length,
				new,
			));
			Ok(())
		}

		/// Set the maximal amount a collator can stake. Existing stakes are not
		/// changed.
		///
		/// The dispatch origin must be Root.
		///
		/// Emits `MaxCandidateStakeChanged`.
		#[pallet::weight(<T as Config>::WeightInfo::set_max_candidate_stake())]
		pub fn set_max_candidate_stake(origin: OriginFor<T>, new: BalanceOf<T>) -> DispatchResult {
			ensure_root(origin)?;
			ensure!(new >= T::MinCollatorCandidateStake::get(), Error::<T>::CannotSetBelowMin);

			// *** No Fail beyond this point ***

			MaxCollatorCandidateStake::<T>::put(new);

			Self::deposit_event(Event::MaxCandidateStakeChanged(new));
			Ok(())
		}

		/// Forcedly removes a collator candidate from the TopCandidates and
		/// clears all associated storage for the candidate and their
		/// delegators.
		///
		/// Prepares unstaking of the candidates and their delegators stake
		/// which can be unlocked via `unlock_unstaked` after waiting at
		/// least `StakeDuration` many blocks. Also increments rewards for the
		/// collator and their delegators.
		///
		/// Increments rewards of candidate and their delegators.
		///
		/// Emits `CandidateRemoved`.
		#[pallet::weight(<T as Config>::WeightInfo::force_remove_candidate(
			T::MaxTopCandidates::get(),
			T::MaxDelegatorsPerCollator::get()
		))]
		pub fn force_remove_candidate(
			origin: OriginFor<T>,
			collator: <T::Lookup as StaticLookup>::Source,
		) -> DispatchResultWithPostInfo {
			ensure_root(origin)?;
			let collator = T::Lookup::lookup(collator)?;
			let state = CandidatePool::<T>::get(&collator).ok_or(Error::<T>::CandidateNotFound)?;
			let total_amount = state.total;

			let mut candidates = TopCandidates::<T>::get();
			ensure!(
				candidates.len().saturated_into::<u32>() > T::MinRequiredCollators::get(),
				Error::<T>::TooFewCollatorCandidates
			);

			// *** No Fail except during remove_candidate beyond this point ***

			// remove candidate storage and increment rewards
			Self::remove_candidate(&collator, &state)?;

			let (num_collators, num_delegators) = if candidates
				.remove(&Stake { owner: collator.clone(), amount: state.total })
				.is_some()
			{
				// update top candidates
				TopCandidates::<T>::put(candidates);
				// update total amount at stake from scratch
				Self::update_total_stake()
			} else {
				(0u32, 0u32)
			};

			Self::deposit_event(Event::CollatorRemoved(collator, total_amount));

			Ok(Some(<T as Config>::WeightInfo::force_remove_candidate(
				num_collators,
				num_delegators,
			))
			.into())
		}

		/// Join the set of collator candidates.
		///
		/// In the next blocks, if the collator candidate has enough funds
		/// staked to be included in any of the top `MaxSelectedCandidates`
		/// positions, it will be included in the set of potential authors that
		/// will be selected by the stake-weighted random selection function.
		///
		/// The staked funds of the new collator candidate are added to the
		/// total stake of the system.
		///
		/// The total amount of funds staked must be within the allowed range as
		/// set in the pallet's configuration.
		///
		/// The dispatch origin must not be already part of the collator
		/// candidates nor of the delegators set.
		///
		/// Emits `JoinedCollatorCandidates`.
		#[pallet::weight(<T as pallet::Config>::WeightInfo::join_candidates(
			T::MaxTopCandidates::get(),
			T::MaxDelegatorsPerCollator::get()
		))]
		pub fn join_candidates(
			origin: OriginFor<T>,
			stake: BalanceOf<T>,
		) -> DispatchResultWithPostInfo {
			let sender = ensure_signed(origin)?;
			if let Some(is_active_candidate) = Self::is_active_candidate(&sender) {
				ensure!(is_active_candidate, Error::<T>::AlreadyLeaving);
				ensure!(!is_active_candidate, Error::<T>::CandidateExists);
			}
			ensure!(!Self::is_delegator(&sender), Error::<T>::DelegatorExists);
			ensure!(stake >= T::MinCollatorCandidateStake::get(), Error::<T>::ValStakeBelowMin);
			ensure!(stake <= MaxCollatorCandidateStake::<T>::get(), Error::<T>::ValStakeAboveMax);
			ensure!(
				Unstaking::<T>::get(&sender).len().saturated_into::<u32>() <
					T::MaxUnstakeRequests::get(),
				Error::<T>::CannotJoinBeforeUnlocking
			);

			// *** No Fail except during increase_lock beyond this point ***

			Self::increase_lock(&sender, stake, BalanceOf::<T>::zero())?;

			let candidate = Candidate::new(sender.clone(), stake);
			let n = Self::update_top_candidates(
				sender.clone(),
				BalanceOf::<T>::zero(),
				BalanceOf::<T>::zero(),
				stake,
				BalanceOf::<T>::zero(),
			);
			CandidatePool::<T>::insert(&sender, candidate);

			Self::deposit_event(Event::JoinedCollatorCandidates(sender, stake));
			Ok(Some(<T as pallet::Config>::WeightInfo::join_candidates(
				n,
				T::MaxDelegatorsPerCollator::get(),
			))
			.into())
		}

		/// Request to leave the set of collator candidates.
		///
		/// On success, the account is immediately removed from the candidate
		/// pool to prevent selection as a collator in future validation rounds,
		/// but unstaking of the funds is executed with a delay of
		/// `StakeDuration` blocks.
		///
		/// The exit request can be reversed by calling
		/// `cancel_leave_candidates`.
		///
		/// This operation affects the pallet's total stake amount. It is
		/// updated even though the funds of the candidate who signaled to leave
		/// are still locked for `ExitDelay` + `StakeDuration` more blocks.
		///
		/// NOTE 1: Upon starting a new session_i in `new_session`, the current
		/// top candidates are selected to be block authors for session_i+1. Any
		/// changes to the top candidates afterwards do not effect the set of
		/// authors for session_i+1.
		/// Thus, we have to make sure none of these collators can
		/// leave before session_i+1 ends by delaying their
		/// exit for `ExitDelay` many blocks.
		///
		/// NOTE 2: We do not increment rewards in this extrinsic as the
		/// candidate could still author blocks, and thus be eligible to receive
		/// rewards, until the end of the next session.
		///
		/// Emits `CollatorScheduledExit`.
		#[pallet::weight(<T as pallet::Config>::WeightInfo::init_leave_candidates(
			T::MaxTopCandidates::get(),
			T::MaxTopCandidates::get().saturating_mul(T::MaxDelegatorsPerCollator::get())
		))]
		pub fn init_leave_candidates(origin: OriginFor<T>) -> DispatchResultWithPostInfo {
			let collator = ensure_signed(origin)?;
			let mut state =
				CandidatePool::<T>::get(&collator).ok_or(Error::<T>::CandidateNotFound)?;
			ensure!(!state.is_leaving(), Error::<T>::AlreadyLeaving);
			let mut candidates = TopCandidates::<T>::get();
			ensure!(
				candidates.len().saturated_into::<u32>() > T::MinRequiredCollators::get(),
				Error::<T>::TooFewCollatorCandidates
			);

			let now = Round::<T>::get().current;
			let when = now.saturating_add(T::ExitQueueDelay::get());
			state.leave_candidates(when);

			// *** No Fail beyond this point ***

			let (num_collators, num_delegators) = if candidates
				.remove(&Stake { owner: collator.clone(), amount: state.total })
				.is_some()
			{
				// update top candidates
				TopCandidates::<T>::put(candidates);
				Self::deposit_event(Event::LeftTopCandidates(collator.clone()));
				// update total amount at stake from scratch
				Self::update_total_stake()
			} else {
				(0u32, 0u32)
			};
			CandidatePool::<T>::insert(&collator, state);

			Self::deposit_event(Event::CollatorScheduledExit(now, collator, when));
			Ok(Some(<T as pallet::Config>::WeightInfo::init_leave_candidates(
				num_collators,
				num_delegators,
			))
			.into())
		}

		/// Execute the network exit of a candidate who requested to leave at
		/// least `ExitQueueDelay` rounds ago. Prepares unstaking of the
		/// candidates and their delegators stake which can be unlocked via
		/// `unlock_unstaked` after waiting at least `StakeDuration` many
		/// blocks.
		///
		/// Requires the candidate to previously have called
		/// `init_leave_candidates`.
		///
		/// The exit request can be reversed by calling
		/// `cancel_leave_candidates`.
		///
		/// NOTE: Iterates over CandidatePool for each candidate over their
		/// delegators to set rewards. Needs to be improved when scaling up
		/// `MaxTopCandidates`.
		///
		/// Emits `CollatorLeft`.
		#[pallet::weight(<T as pallet::Config>::WeightInfo::execute_leave_candidates(
			T::MaxTopCandidates::get(),
			T::MaxDelegatorsPerCollator::get(),
		))]
		pub fn execute_leave_candidates(
			origin: OriginFor<T>,
			collator: <T::Lookup as StaticLookup>::Source,
		) -> DispatchResultWithPostInfo {
			ensure_signed(origin)?;
			let collator = T::Lookup::lookup(collator)?;
			let state = CandidatePool::<T>::get(&collator).ok_or(Error::<T>::CandidateNotFound)?;
			ensure!(state.is_leaving(), Error::<T>::NotLeaving);
			ensure!(state.can_exit(Round::<T>::get().current), Error::<T>::CannotLeaveYet);

			let num_delegators = state.delegators.len().saturated_into::<u32>();
			let total_amount = state.total;

			// *** No Fail except during remove_candidate beyond this point ***

			// remove candidate storage and increment rewards
			Self::remove_candidate(&collator, &state)?;

			Self::deposit_event(Event::CandidateLeft(collator, total_amount));

			Ok(Some(<T as pallet::Config>::WeightInfo::execute_leave_candidates(
				T::MaxTopCandidates::get(),
				num_delegators,
			))
			.into())
		}

		/// Revert the previously requested exit of the network of a collator
		/// candidate. On success, adds back the candidate to the TopCandidates
		/// and updates the collators.
		///
		/// Requires the candidate to previously have called
		/// `init_leave_candidates`.
		///
		/// Emits `CollatorCanceledExit`.
		#[pallet::weight(<T as pallet::Config>::WeightInfo::cancel_leave_candidates(
			T::MaxTopCandidates::get(),
			T::MaxDelegatorsPerCollator::get(),
		))]
		pub fn cancel_leave_candidates(origin: OriginFor<T>) -> DispatchResultWithPostInfo {
			let candidate = ensure_signed(origin)?;
			let mut state =
				CandidatePool::<T>::get(&candidate).ok_or(Error::<T>::CandidateNotFound)?;
			ensure!(state.is_leaving(), Error::<T>::NotLeaving);

			// revert leaving state
			state.revert_leaving();

			// *** No Fail beyond this point ***

			let n = Self::update_top_candidates(
				candidate.clone(),
				state.stake,
				// safe because total >= stake
				state.total - state.stake,
				state.stake,
				state.total - state.stake,
			);

			// update candidates for next round
			CandidatePool::<T>::insert(&candidate, state);

			Self::deposit_event(Event::CollatorCanceledExit(candidate));

			Ok(Some(<T as pallet::Config>::WeightInfo::cancel_leave_candidates(
				n,
				T::MaxDelegatorsPerCollator::get(),
			))
			.into())
		}

		/// Stake more funds for a collator candidate.
		///
		/// If not in the set of candidates, staking enough funds allows the
		/// account to be added to it. The larger amount of funds, the higher
		/// chances to be selected as the author of the next block.
		///
		/// This operation affects the pallet's total stake amount.
		///
		/// The resulting total amount of funds staked must be within the
		/// allowed range as set in the pallet's configuration.
		///
		/// Emits `CollatorStakedMore`.
		#[pallet::weight(<T as pallet::Config>::WeightInfo::candidate_stake_more(
			T::MaxTopCandidates::get(),
			T::MaxDelegatorsPerCollator::get(),
			T::MaxUnstakeRequests::get().saturated_into::<u32>()
		))]
		pub fn candidate_stake_more(
			origin: OriginFor<T>,
			more: BalanceOf<T>,
		) -> DispatchResultWithPostInfo {
			let collator = ensure_signed(origin)?;

			ensure!(!more.is_zero(), Error::<T>::ValStakeZero);
			let mut state =
				CandidatePool::<T>::get(&collator).ok_or(Error::<T>::CandidateNotFound)?;
			ensure!(!state.is_leaving(), Error::<T>::CannotStakeIfLeaving);

			let CandidateOf::<T, _> { stake: before_stake, total: before_total, .. } = state;
			state.stake_more(more);
			let after_stake = state.stake;
			ensure!(
				state.stake <= MaxCollatorCandidateStake::<T>::get(),
				Error::<T>::ValStakeAboveMax
			);

			// *** No Fail except during increase_lock beyond this point ***

			let unstaking_len = Self::increase_lock(&collator, state.stake, more)?;

			let n = if state.is_active() {
				Self::update_top_candidates(
					collator.clone(),
					before_stake,
					// safe because total >= stake
					before_total - before_stake,
					state.stake,
					state.total - state.stake,
				)
			} else {
				0u32
			};
			CandidatePool::<T>::insert(&collator, state);

			// increment rewards for collator and update number of rewarded blocks
			Self::do_inc_collator_reward(&collator, before_stake);

			Self::deposit_event(Event::CollatorStakedMore(collator, before_stake, after_stake));
			Ok(Some(<T as pallet::Config>::WeightInfo::candidate_stake_more(
				n,
				T::MaxDelegatorsPerCollator::get(),
				unstaking_len,
			))
			.into())
		}

		/// Stake less funds for a collator candidate.
		///
		/// If the new amount of staked fund is not large enough, the account
		/// could be removed from the set of collator candidates and not be
		/// considered for authoring the next blocks.
		///
		/// This operation affects the pallet's total stake amount.
		///
		/// The unstaked funds are not released immediately to the account, but
		/// they will be available after `StakeDuration` blocks.
		///
		/// The resulting total amount of funds staked must be within the
		/// allowed range as set in the pallet's configuration.
		///
		/// Emits `CollatorStakedLess`.
		#[pallet::weight(<T as pallet::Config>::WeightInfo::candidate_stake_less(
			T::MaxTopCandidates::get(),
			T::MaxDelegatorsPerCollator::get()
		))]
		pub fn candidate_stake_less(
			origin: OriginFor<T>,
			less: BalanceOf<T>,
		) -> DispatchResultWithPostInfo {
			let collator = ensure_signed(origin)?;
			ensure!(!less.is_zero(), Error::<T>::ValStakeZero);

			let mut state =
				CandidatePool::<T>::get(&collator).ok_or(Error::<T>::CandidateNotFound)?;
			ensure!(!state.is_leaving(), Error::<T>::CannotStakeIfLeaving);

			let CandidateOf::<T, _> { stake: before_stake, total: before_total, .. } = state;
			let after = state.stake_less(less).ok_or(Error::<T>::Underflow)?;
			ensure!(after >= T::MinCollatorCandidateStake::get(), Error::<T>::ValStakeBelowMin);

			// *** No Fail except during prep_unstake beyond this point ***

			// we don't unlock immediately
			Self::prep_unstake(&collator, less, false)?;

			let n = if state.is_active() {
				Self::update_top_candidates(
					collator.clone(),
					before_stake,
					// safe because total >= stake
					before_total - before_stake,
					state.stake,
					state.total - state.stake,
				)
			} else {
				0u32
			};
			CandidatePool::<T>::insert(&collator, state);

			Self::deposit_event(Event::CollatorStakedLess(collator, before_stake, after));
			Ok(Some(<T as pallet::Config>::WeightInfo::candidate_stake_less(
				n,
				T::MaxDelegatorsPerCollator::get(),
			))
			.into())
		}

		/// Join the set of delegators by delegating to a collator candidate.
		///
		/// The account that wants to delegate cannot be part of the collator
		/// candidates set as well.
		///
		/// The caller must _not_ have a delegation. If that is the case, they
		/// are required to first remove the delegation.
		///
		/// The amount staked must be larger than the minimum required to become
		/// a delegator as set in the pallet's configuration.
		///
		/// As only `MaxDelegatorsPerCollator` are allowed to delegate a given
		/// collator, the amount staked must be larger than the lowest one in
		/// the current set of delegator for the operation to be meaningful.
		///
		/// The collator's total stake as well as the pallet's total stake are
		/// increased accordingly.
		///
		/// Emits `Delegation`.
		/// Emits `DelegationReplaced` if the candidate has
		/// `MaxDelegatorsPerCollator` many delegations but this delegator
		/// staked more than one of the other delegators of this candidate.
		#[pallet::weight(<T as pallet::Config>::WeightInfo::join_delegators(
			T::MaxTopCandidates::get(),
			T::MaxDelegatorsPerCollator::get()
		))]
		pub fn join_delegators(
			origin: OriginFor<T>,
			collator: <T::Lookup as StaticLookup>::Source,
			amount: BalanceOf<T>,
		) -> DispatchResultWithPostInfo {
			let acc = ensure_signed(origin)?;
			let collator = T::Lookup::lookup(collator)?;

			// check balance
			ensure!(
				pallet_balances::Pallet::<T>::free_balance(acc.clone()) >= amount.into(),
				pallet_balances::Error::<T>::InsufficientBalance
			);

			// first delegation
			ensure!(DelegatorState::<T>::get(&acc).is_none(), Error::<T>::AlreadyDelegating);
			ensure!(amount >= T::MinDelegatorStake::get(), Error::<T>::DelegationBelowMin);

			// cannot be a collator candidate and delegator with same AccountId
			ensure!(Self::is_active_candidate(&acc).is_none(), Error::<T>::CandidateExists);
			ensure!(
				Unstaking::<T>::get(&acc).len().saturated_into::<u32>() <
					T::MaxUnstakeRequests::get(),
				Error::<T>::CannotJoinBeforeUnlocking
			);
			// cannot delegate if number of delegations in this round exceeds
			// MaxDelegationsPerRound
			let delegation_counter = Self::get_delegation_counter(&acc)?;

			// prepare update of collator state
			let mut state =
				CandidatePool::<T>::get(&collator).ok_or(Error::<T>::CandidateNotFound)?;
			let num_delegations_pre_insertion: u32 = state.delegators.len().saturated_into();

			ensure!(!state.is_leaving(), Error::<T>::CannotDelegateIfLeaving);
			let delegation = Stake { owner: acc.clone(), amount };

			// attempt to insert delegator and check for uniqueness
			// NOTE: excess is handled below because we support replacing a delegator with
			// fewer stake
			let insert_delegator = state
				.delegators
				// we handle TooManyDelegators error below in do_update_delegator
				.try_insert(delegation.clone())
				.unwrap_or(true);
			// should never fail but let's be safe
			ensure!(insert_delegator, Error::<T>::DelegatorExists);

			let delegator_state = Delegator { amount, owner: collator.clone() };
			let CandidateOf::<T, _> { stake: old_stake, total: old_total, .. } = state;

			// update state and potentially prepare kicking a delegator with less staked
			// amount (includes setting rewards for kicked delegator)
			let state = if num_delegations_pre_insertion == T::MaxDelegatorsPerCollator::get() {
				Self::do_update_delegator(delegation, state)?
			} else {
				state.total = state.total.saturating_add(amount);
				state
			};
			let new_total = state.total;

			// *** No Fail except during increase_lock beyond this point ***

			// lock stake
			Self::increase_lock(&acc, amount, BalanceOf::<T>::zero())?;

			// update top candidates and total amount at stake
			let n = if state.is_active() {
				Self::update_top_candidates(
					collator.clone(),
					old_stake,
					// safe because total >= stake
					old_total - old_stake,
					state.stake,
					state.total - state.stake,
				)
			} else {
				0u32
			};

			// update states
			CandidatePool::<T>::insert(&collator, state);
			DelegatorState::<T>::insert(&acc, delegator_state);
			LastDelegation::<T>::insert(&acc, delegation_counter);

			// initiate rewarded counter to match the current authored counter of the
			// candidate
			BlocksRewarded::<T>::insert(&acc, BlocksAuthored::<T>::get(&collator));

			Self::deposit_event(Event::Delegation(acc, amount, collator, new_total));
			Ok(Some(<T as pallet::Config>::WeightInfo::join_delegators(
				n,
				T::MaxDelegatorsPerCollator::get(),
			))
			.into())
		}

		/// Leave the set of delegators and, by implication, revoke all ongoing
		/// delegations.
		///
		/// All staked funds are not unlocked immediately, but they are added to
		/// the queue of pending unstaking, and will effectively be released
		/// after `StakeDuration` blocks from the moment the delegator leaves.
		///
		/// This operation reduces the total stake of the pallet as well as the
		/// stakes of all collators that were delegated, potentially affecting
		/// their chances to be included in the set of candidates in the next
		/// rounds.
		///
		/// Automatically increments the accumulated rewards of the origin of
		/// the current delegation.
		///
		/// Emits `DelegatorLeft`.
		#[pallet::weight(<T as pallet::Config>::WeightInfo::leave_delegators(
			T::MaxTopCandidates::get(),
			T::MaxDelegatorsPerCollator::get()
		))]
		pub fn leave_delegators(origin: OriginFor<T>) -> DispatchResultWithPostInfo {
			let acc = ensure_signed(origin)?;
			let delegator = DelegatorState::<T>::get(&acc).ok_or(Error::<T>::DelegatorNotFound)?;
			let collator = delegator.owner;
			Self::delegator_leaves_collator(acc.clone(), collator)?;

			// *** No Fail beyond this point ***

			DelegatorState::<T>::remove(&acc);

			Self::deposit_event(Event::DelegatorLeft(acc, delegator.amount));
			Ok(Some(<T as pallet::Config>::WeightInfo::leave_delegators(
				1,
				T::MaxDelegatorsPerCollator::get(),
			))
			.into())
		}

		/// Increase the stake for delegating a collator candidate.
		///
		/// If not in the set of candidates, staking enough funds allows the
		/// collator candidate to be added to it.
		///
		/// Emits `DelegatorStakedMore`.
		#[pallet::weight(<T as pallet::Config>::WeightInfo::delegator_stake_more(
			T::MaxTopCandidates::get(),
			T::MaxDelegatorsPerCollator::get(),
			T::MaxUnstakeRequests::get().saturated_into::<u32>())
		)]
		pub fn delegator_stake_more(
			origin: OriginFor<T>,
			more: BalanceOf<T>,
		) -> DispatchResultWithPostInfo {
			let delegator = ensure_signed(origin)?;
			ensure!(!more.is_zero(), Error::<T>::ValStakeZero);

			let mut delegation =
				DelegatorState::<T>::get(&delegator).ok_or(Error::<T>::DelegatorNotFound)?;
			let candidate = delegation.owner.clone();
			let mut collator =
				CandidatePool::<T>::get(&candidate).ok_or(Error::<T>::CandidateNotFound)?;
			ensure!(!collator.is_leaving(), Error::<T>::CannotDelegateIfLeaving);
			let stake_after = delegation
				.try_increment(candidate.clone(), more)
				.map_err(|_| Error::<T>::DelegationNotFound)?;

			// *** No Fail except during increase_lock beyond this point ***

			// update lock
			let unstaking_len = Self::increase_lock(&delegator, stake_after, more)?;

			let CandidateOf::<T, _> { stake: before_stake, total: before_total, .. } = collator;
			collator.inc_delegator(delegator.clone(), more);
			let after = collator.total;

			// update top candidates and total amount at stake
			let n = if collator.is_active() {
				Self::update_top_candidates(
					candidate.clone(),
					before_stake,
					// safe because total >= stake
					before_total - before_stake,
					collator.stake,
					collator.total - collator.stake,
				)
			} else {
				0u32
			};

			// increment rewards and update number of rewarded blocks
			Self::do_inc_delegator_reward(&delegator, stake_after.saturating_sub(more), &candidate);

			CandidatePool::<T>::insert(&candidate, collator);
			DelegatorState::<T>::insert(&delegator, delegation);

			Self::deposit_event(Event::DelegatorStakedMore(
				delegator,
				candidate,
				before_total,
				after,
			));
			Ok(Some(<T as pallet::Config>::WeightInfo::delegator_stake_more(
				n,
				T::MaxDelegatorsPerCollator::get(),
				unstaking_len,
			))
			.into())
		}

		/// Reduce the stake for delegating a collator candidate.
		///
		/// If the new amount of staked fund is not large enough, the collator
		/// could be removed from the set of collator candidates and not be
		/// considered for authoring the next blocks.
		///
		/// The unstaked funds are not release immediately to the account, but
		/// they will be available after `StakeDuration` blocks.
		///
		/// The remaining staked funds must still be larger than the minimum
		/// required by this pallet to maintain the status of delegator.
		///
		/// The resulting total amount of funds staked must be within the
		/// allowed range as set in the pallet's configuration.
		///
		/// Emits `DelegatorStakedLess`.
		#[pallet::weight(<T as pallet::Config>::WeightInfo::delegator_stake_less(
			T::MaxTopCandidates::get(),
			T::MaxDelegatorsPerCollator::get()
		))]
		pub fn delegator_stake_less(
			origin: OriginFor<T>,
			less: BalanceOf<T>,
		) -> DispatchResultWithPostInfo {
			let delegator = ensure_signed(origin)?;
			ensure!(!less.is_zero(), Error::<T>::ValStakeZero);

			let mut delegation =
				DelegatorState::<T>::get(&delegator).ok_or(Error::<T>::DelegatorNotFound)?;
			let candidate = delegation.owner.clone();
			let mut collator =
				CandidatePool::<T>::get(&candidate).ok_or(Error::<T>::CandidateNotFound)?;
			ensure!(!collator.is_leaving(), Error::<T>::CannotDelegateIfLeaving);
			let stake_after = delegation
				.try_decrement(candidate.clone(), less)
				.map_err(|_| Error::<T>::DelegationNotFound)?
				.ok_or(Error::<T>::Underflow)?;

			ensure!(stake_after >= T::MinDelegatorStake::get(), Error::<T>::DelegationBelowMin);

			// *** No Fail except during prep_unstake beyond this point ***

			Self::prep_unstake(&delegator, less, false)?;

			let CandidateOf::<T, _> { stake: before_stake, total: before_total, .. } = collator;
			collator.dec_delegator(delegator.clone(), less);
			let after = collator.total;

			// update top candidates and total amount at stake
			let n = if collator.is_active() {
				Self::update_top_candidates(
					candidate.clone(),
					before_stake,
					// safe because total >= stake
					before_total - before_stake,
					collator.stake,
					collator.total - collator.stake,
				)
			} else {
				0u32
			};

			// increment rewards and update number of rewarded blocks
			Self::do_inc_delegator_reward(&delegator, stake_after.saturating_add(less), &candidate);

			CandidatePool::<T>::insert(&candidate, collator);
			DelegatorState::<T>::insert(&delegator, delegation);

			Self::deposit_event(Event::DelegatorStakedLess(
				delegator,
				candidate,
				before_total,
				after,
			));
			Ok(Some(<T as pallet::Config>::WeightInfo::delegator_stake_less(
				n,
				T::MaxDelegatorsPerCollator::get(),
			))
			.into())
		}

		/// Unlock all previously staked funds that are now available for
		/// unlocking by the origin account after `StakeDuration` blocks have
		/// elapsed.
		///
		/// Weight: O(U) where U is the number of locked unstaking requests
		/// bounded by `MaxUnstakeRequests`.
		/// - Reads: [Origin Account], Unstaking, Locks
		/// - Writes: Unstaking, Locks
		/// - Kills: Unstaking & Locks if no balance is locked anymore
		/// # </weight>
		#[pallet::weight(<T as pallet::Config>::WeightInfo::unlock_unstaked(
			T::MaxUnstakeRequests::get().saturated_into::<u32>()
		))]
		pub fn unlock_unstaked(
			origin: OriginFor<T>,
			target: <T::Lookup as StaticLookup>::Source,
		) -> DispatchResultWithPostInfo {
			ensure_signed(origin)?;
			let target = T::Lookup::lookup(target)?;

			let unstaking_len = Self::do_unlock(&target)?;

			Ok(Some(<T as pallet::Config>::WeightInfo::unlock_unstaked(unstaking_len)).into())
		}

		/// Claim block authoring rewards for the target address.
		///
		/// Requires `Rewards` to be set beforehand, which can by triggered by
		/// any of the following options
		/// * Calling increment_{collator, delegator}_rewards (active)
		/// * Altering your stake (active)
		/// * Leaving the network as a collator (active)
		/// * Revoking a delegation as a delegator (active)
		/// * Being a delegator whose collator left the network, altered their stake or incremented
		///   rewards (passive)
		///
		/// The dispatch origin can be any signed one, e.g., anyone can claim
		/// for anyone.
		///
		/// Emits `Rewarded`.
		#[pallet::weight(<T as Config>::WeightInfo::claim_rewards())]
		pub fn claim_rewards(origin: OriginFor<T>) -> DispatchResult {
			let target = ensure_signed(origin)?;

			// reset rewards
			let rewards = Rewards::<T>::take(&target);
			ensure!(!rewards.is_zero(), Error::<T>::RewardsNotFound);

			// mint into target
			let rewards = T::Currency::deposit_into_existing(&target, rewards)?;

			Self::deposit_event(Event::Rewarded(target, rewards.peek()));

			Ok(())
		}

		/// Actively increment the rewards of a collator.
		///
		/// The same effect is triggered by changing the stake or leaving the
		/// network.
		///
		/// The dispatch origin must be a collator.
		#[pallet::weight(<T as Config>::WeightInfo::increment_collator_rewards())]
		pub fn increment_collator_rewards(origin: OriginFor<T>) -> DispatchResult {
			let collator = ensure_signed(origin)?;
			let state = CandidatePool::<T>::get(&collator).ok_or(Error::<T>::CandidateNotFound)?;

			// increment rewards and update number of rewarded blocks
			Self::do_inc_collator_reward(&collator, state.stake);

			Ok(())
		}

		/// Actively increment the rewards of a delegator.
		///
		/// The same effect is triggered by changing the stake or revoking
		/// delegations.
		///
		/// The dispatch origin must be a delegator.
		#[pallet::weight(<T as Config>::WeightInfo::increment_delegator_rewards())]
		pub fn increment_delegator_rewards(origin: OriginFor<T>) -> DispatchResult {
			let delegator = ensure_signed(origin)?;
			let delegation =
				DelegatorState::<T>::get(&delegator).ok_or(Error::<T>::DelegatorNotFound)?;
			let collator = delegation.owner;

			// increment rewards and update number of rewarded blocks
			Self::do_inc_delegator_reward(&delegator, delegation.amount, &collator);

			Ok(())
		}
	}

	impl<T: Config> Pallet<T> {
		/// Check whether an account is currently delegating.
		pub fn is_delegator(acc: &T::AccountId) -> bool {
			DelegatorState::<T>::get(acc).is_some()
		}

		/// Check whether an account is currently a collator candidate and
		/// whether their state is CollatorStatus::Active.
		///
		/// Returns Some(is_active) if the account is a candidate, else None.
		pub fn is_active_candidate(acc: &T::AccountId) -> Option<bool> {
			if let Some(state) = CandidatePool::<T>::get(acc) {
				Some(state.status == CandidateStatus::Active)
			} else {
				None
			}
		}

		/// Update the top candidates and total amount at stake after mutating
		/// an active candidate's stake.
		///
		/// NOTE: It is assumed that the calling context checks whether the
		/// collator candidate is currently active before calling this function.
		fn update_top_candidates(
			candidate: T::AccountId,
			old_self: BalanceOf<T>,
			old_delegators: BalanceOf<T>,
			new_self: BalanceOf<T>,
			new_delegators: BalanceOf<T>,
		) -> u32 {
			let mut top_candidates = TopCandidates::<T>::get();
			let num_top_candidates: u32 = top_candidates.len().saturated_into();
			let old_stake =
				Stake { owner: candidate.clone(), amount: old_self.saturating_add(old_delegators) };
			let new_stake =
				Stake { owner: candidate.clone(), amount: new_self.saturating_add(new_delegators) };

			// update TopCandidates set
			let maybe_top_candidate_update = if let Ok(i) = top_candidates.linear_search(&old_stake)
			{
				// case 1: candidate is member of TopCandidates with old stake
				top_candidates.mutate(|vec| {
					if let Some(stake) = vec.get_mut(i) {
						stake.amount = new_stake.amount;
					}
				});
				Some((Some(i), top_candidates))
			} else if top_candidates.try_insert_replace(new_stake.clone()).is_ok() {
				// case 2: candidate ascends into TopCandidates with new stake
				// and might replace another candidate if TopCandidates is full
				Self::deposit_event(Event::EnteredTopCandidates(candidate));
				Some((None, top_candidates))
			} else {
				// case 3: candidate neither was nor will be member of TopCandidates
				None
			};

			// update storage for TotalCollatorStake and TopCandidates
			if let Some((maybe_old_idx, top_candidates)) = maybe_top_candidate_update {
				let max_selected_candidates =
					MaxSelectedCandidates::<T>::get().saturated_into::<usize>();
				let was_collating =
					maybe_old_idx.map(|i| i < max_selected_candidates).unwrap_or(false);
				let is_collating = top_candidates
					.linear_search(&new_stake)
					.map(|i| i < max_selected_candidates)
					.unwrap_or(false);

				// update TopCollatorStake storage iff candidate was or will be a collator
				match (was_collating, is_collating) {
					(true, true) => {
						Self::update_total_stake_by(
							new_self,
							new_delegators,
							old_self,
							old_delegators,
						);
					},
					(true, false) => {
						// candidate left the collator set because they staked less and have been
						// replaced by the next candidate in the queue at position
						// min(max_selected_candidates, top_candidates) - 1 in TopCandidates
						let new_col_idx =
							max_selected_candidates.min(top_candidates.len()).saturating_sub(1);

						// get displacer
						let (add_collators, add_delegators) =
							Self::get_top_candidate_stake_at(&top_candidates, new_col_idx)
								// shouldn't be possible to fail, but we handle it gracefully
								.unwrap_or((new_self, new_delegators));
						Self::update_total_stake_by(
							add_collators,
							add_delegators,
							old_self,
							old_delegators,
						);
					},
					(false, true) => {
						// candidate pushed out the least staked collator which is now at position
						// min(max_selected_top_candidates, top_candidates - 1) in TopCandidates
						let old_col_idx =
							max_selected_candidates.min(top_candidates.len().saturating_sub(1));

						// get amount to subtract from TotalCollatorStake
						let (drop_self, drop_delegators) =
							Self::get_top_candidate_stake_at(&top_candidates, old_col_idx)
								// default to zero if candidate DNE, e.g. TopCandidates is not full
								.unwrap_or((BalanceOf::<T>::zero(), BalanceOf::<T>::zero()));
						Self::update_total_stake_by(
							new_self,
							new_delegators,
							drop_self,
							drop_delegators,
						);
					},
					_ => {},
				}

				// update TopCandidates storage
				TopCandidates::<T>::put(top_candidates);
			}

			num_top_candidates
		}

		/// Retrieve the staked amounts (self, sum of delegators) of member of
		/// [TopCandidates] at the given index, if it exists.
		fn get_top_candidate_stake_at(
			top_candidates: &OrderedSet<StakeOf<T>, T::MaxTopCandidates>,
			index: usize,
		) -> Option<(BalanceOf<T>, BalanceOf<T>)> {
			top_candidates
				.get(index)
				.and_then(|stake| CandidatePool::<T>::get(&stake.owner))
				// SAFETY: the total is always more than the stake
				.map(|state| (state.stake, state.total - state.stake))
		}

		/// Mutate the [TotalCollatorStake] by both incrementing and decreasing
		/// it by the provided values.
		fn update_total_stake_by(
			add_collators: BalanceOf<T>,
			add_delegators: BalanceOf<T>,
			sub_collators: BalanceOf<T>,
			sub_delegators: BalanceOf<T>,
		) {
			TotalCollatorStake::<T>::mutate(|total| {
				total.collators =
					total.collators.saturating_sub(sub_collators).saturating_add(add_collators);
				total.delegators =
					total.delegators.saturating_sub(sub_delegators).saturating_add(add_delegators);
			});
		}

		/// Iterate over the top `MaxSelectedCandidates` many collators in terms
		/// of cumulated stake (self + from delegators) from the [TopCandidates]
		/// and recalculate the [TotalCollatorStake] from scratch.
		///
		/// NOTE: Should only be called in rare circumstances in which we cannot
		/// guarantee a single candidate's stake has changed, e.g. on genesis or
		/// when a collator leaves. Otherwise, please use
		/// [update_total_stake_by].
		fn update_total_stake() -> (u32, u32) {
			let mut num_of_delegators = 0u32;
			let mut collator_stake = BalanceOf::<T>::zero();
			let mut delegator_stake = BalanceOf::<T>::zero();

			let collators = Self::selected_candidates();

			// Snapshot exposure for round for weighting reward distribution
			for account in collators.iter() {
				let state = CandidatePool::<T>::get(account)
					.expect("all members of TopCandidates must be candidates q.e.d");
				num_of_delegators =
					num_of_delegators.max(state.delegators.len().saturated_into::<u32>());

				// sum up total stake and amount of collators, delegators
				let amount_collator = state.stake;
				collator_stake = collator_stake.saturating_add(state.stake);
				// safe to subtract because total >= stake
				let amount_delegators = state.total - amount_collator;
				delegator_stake = delegator_stake.saturating_add(amount_delegators);
			}

			TotalCollatorStake::<T>::mutate(|total| {
				total.collators = collator_stake;
				total.delegators = delegator_stake;
			});

			// return number of selected candidates and the corresponding number of their
			// delegators for post-weight correction
			(collators.len().saturated_into(), num_of_delegators)
		}

		/// Update the collator's state by removing the delegator's stake and
		/// starting the process to unlock the delegator's staked funds as well
		/// as incrementing their accumulated rewards.
		///
		/// This operation affects the pallet's total stake.
		fn delegator_leaves_collator(
			delegator: T::AccountId,
			collator: T::AccountId,
		) -> DispatchResult {
			let mut state =
				CandidatePool::<T>::get(&collator).ok_or(Error::<T>::CandidateNotFound)?;

			let delegator_stake = state
				.delegators
				.remove(&Stake {
					owner: delegator.clone(),
					// amount is irrelevant for removal
					amount: BalanceOf::<T>::one(),
				})
				.map(|nom| nom.amount)
				.ok_or(Error::<T>::DelegatorNotFound)?;

			let CandidateOf::<T, _> { stake: old_stake, total: old_total, .. } = state;
			state.total = state.total.saturating_sub(delegator_stake);
			let new_total = state.total;

			// increment rewards and kill storage for number of rewarded blocks
			Self::do_inc_delegator_reward(&delegator, delegator_stake, &collator);
			BlocksRewarded::<T>::remove(&delegator);

			// we don't unlock immediately
			Self::prep_unstake(&delegator, delegator_stake, false)?;

			// update top candidates and total amount at stake
			if state.is_active() {
				Self::update_top_candidates(
					collator.clone(),
					old_stake,
					// safe because total >= stake
					old_total - old_stake,
					state.stake,
					state.total - state.stake,
				);
			}
			CandidatePool::<T>::insert(&collator, state);

			Self::deposit_event(Event::DelegatorLeftCollator(
				delegator,
				collator,
				delegator_stake,
				new_total,
			));
			Ok(())
		}

		/// Return the best `MaxSelectedCandidates` many candidates.
		///
		/// In case a collator from last round was replaced by a candidate with
		/// the same total stake during sorting, we revert this swap to
		/// prioritize collators over candidates.
		pub fn selected_candidates() -> BoundedVec<T::AccountId, T::MaxTopCandidates> {
			let candidates = TopCandidates::<T>::get();

			// Should never fail since WASM usize are 32bits and native are either 32 or 64
			let top_n = MaxSelectedCandidates::<T>::get().saturated_into::<usize>();

			log::trace!("{} Candidates for {} Collator seats", candidates.len(), top_n);

			// Choose the top MaxSelectedCandidates qualified candidates
			let collators = candidates
				.into_iter()
				.take(top_n)
				.filter(|x| x.amount >= T::MinCollatorStake::get())
				.map(|x| x.owner)
				.collect::<Vec<T::AccountId>>();

			collators.try_into().expect("Did not extend Collators q.e.d.")
		}

		/// Attempts to add the stake to the set of delegators of a collator
		/// which already reached its maximum size by removing an already
		/// existing delegator with less staked value. If the given staked
		/// amount is at most the minimum staked value of the original delegator
		/// set, an error is returned.
		///
		/// Sets rewards for the removed delegator.
		///
		/// Returns a tuple which contains the updated candidate state as well
		/// as the potentially replaced delegation which will be used later when
		/// updating the storage of the replaced delegator.
		///
		/// Emits `DelegationReplaced` if the stake exceeds one of the current
		/// delegations.
		#[allow(clippy::type_complexity)]
		fn do_update_delegator(
			stake: Stake<T::AccountId, BalanceOf<T>>,
			mut state: Candidate<T::AccountId, BalanceOf<T>, T::MaxDelegatorsPerCollator>,
		) -> Result<CandidateOf<T, T::MaxDelegatorsPerCollator>, DispatchError> {
			// attempt to replace the last element of the set
			let stake_to_remove =
				state.delegators.try_insert_replace(stake.clone()).map_err(|err_too_many| {
					if err_too_many {
						Error::<T>::TooManyDelegators
					} else {
						// should never occur because we previously check this case, but let's be
						// sure
						Error::<T>::AlreadyDelegating
					}
				})?;

			state.total = state.total.saturating_add(stake.amount);

			if let Some(stake_to_remove) = stake_to_remove {
				// update total stake
				state.total = state.total.saturating_sub(stake_to_remove.amount);

				// update rewards for kicked delegator
				Self::do_inc_delegator_reward(
					&stake_to_remove.owner,
					stake_to_remove.amount,
					&state.id,
				);
				// prepare unstaking for kicked delegator
				Self::prep_unstake(&stake_to_remove.owner, stake_to_remove.amount, true)?;
				// remove Delegator state for kicked delegator
				DelegatorState::<T>::remove(&stake_to_remove.owner);

				Self::deposit_event(Event::DelegationReplaced(
					stake.owner,
					stake.amount,
					stake_to_remove.owner,
					stake_to_remove.amount,
					state.id.clone(),
					state.total,
				));
			}

			Ok(state)
		}

		/// Either set or increase the BalanceLock of target account to
		/// amount.
		///
		/// Consumes unstaked balance which can be unlocked in the future up to
		/// amount and updates `Unstaking` storage accordingly.
		fn increase_lock(
			who: &T::AccountId,
			amount: BalanceOf<T>,
			more: BalanceOf<T>,
		) -> Result<u32, DispatchError> {
			ensure!(
				pallet_balances::Pallet::<T>::free_balance(who) >= amount.into(),
				pallet_balances::Error::<T>::InsufficientBalance
			);

			let mut unstaking_len = 0u32;

			// *** No Fail except during Unstaking mutation beyond this point ***

			// update Unstaking by consuming up to {amount | more}
			Unstaking::<T>::try_mutate(who, |unstaking| -> DispatchResult {
				// reduce {amount | more} by unstaking until either {amount | more} is zero or
				// no unstaking is left
				// if more is set, we only want to reduce by more to achieve 100 - 40 + 30 = 90
				// locked
				let mut amt_consuming_unstaking = if more.is_zero() { amount } else { more };
				unstaking_len = unstaking.len().saturated_into();
				for (block_number, locked_balance) in unstaking.clone() {
					if amt_consuming_unstaking.is_zero() {
						break
					} else if locked_balance > amt_consuming_unstaking {
						// amount is only reducible by locked_balance - amt_consuming_unstaking
						let delta = locked_balance.saturating_sub(amt_consuming_unstaking);
						// replace old entry with delta
						unstaking
							.try_insert(block_number, delta)
							.map_err(|_| Error::<T>::NoMoreUnstaking)?;
						amt_consuming_unstaking = Zero::zero();
					} else {
						// amount is either still reducible or reached
						amt_consuming_unstaking =
							amt_consuming_unstaking.saturating_sub(locked_balance);
						unstaking.remove(&block_number);
					}
				}
				Ok(())
			})?;

			// Either set a new lock or potentially extend the existing one if amount
			// exceeds the currently locked amount
			T::Currency::extend_lock(STAKING_ID, who, amount, WithdrawReasons::all());

			Ok(unstaking_len)
		}

		/// Set the unlocking block for the account and corresponding amount
		/// which can be unlocked via `unlock_unstaked` after waiting at
		/// least for `StakeDuration` many blocks.
		///
		/// Throws if the amount is zero (unlikely) or if active unlocking
		/// requests exceed limit. The latter defends against stake reduction
		/// spamming.
		fn prep_unstake(
			who: &T::AccountId,
			amount: BalanceOf<T>,
			is_removal: bool,
		) -> DispatchResult {
			// should never occur but let's be safe
			ensure!(!amount.is_zero(), Error::<T>::StakeNotFound);

			let now = frame_system::Pallet::<T>::block_number();
			let unlock_block = now.saturating_add(T::StakeDuration::get());
			let mut unstaking = Unstaking::<T>::get(who);

			let allowed_unstakings = if is_removal {
				// the account was forcedly removed and we allow to fill all unstake requests
				T::MaxUnstakeRequests::get()
			} else {
				// we need to reserve a free slot for a forced removal of the account
				T::MaxUnstakeRequests::get().saturating_sub(1)
			};
			ensure!(
				unstaking.len().saturated_into::<u32>() < allowed_unstakings,
				Error::<T>::NoMoreUnstaking,
			);

			// if existent, we have to add the current amount of same unlock_block, because
			// insert overwrites the current value
			let amount = amount
				.saturating_add(*unstaking.get(&unlock_block).unwrap_or(&BalanceOf::<T>::zero()));
			unstaking
				.try_insert(unlock_block, amount)
				.map_err(|_| Error::<T>::NoMoreUnstaking)?;
			Unstaking::<T>::insert(who, unstaking);
			Ok(())
		}

		/// Clear the CandidatePool of the candidate and remove all delegations
		/// to the candidate. Moreover, prepare unstaking for the candidate and
		/// their former delegations.
		fn remove_candidate(
			collator: &T::AccountId,
			state: &CandidateOf<T, T::MaxDelegatorsPerCollator>,
		) -> DispatchResult {
			// iterate over delegators
			for stake in &state.delegators[..] {
				// increment rewards
				Self::do_inc_delegator_reward(&stake.owner, stake.amount, collator);
				// prepare unstaking of delegator
				Self::prep_unstake(&stake.owner, stake.amount, true)?;
				// remove delegation from delegator state
				if let Some(mut delegator) = DelegatorState::<T>::get(&stake.owner) {
					delegator
						.try_clear(collator.clone())
						.map_err(|_| Error::<T>::DelegationNotFound)?;
					DelegatorState::<T>::remove(&stake.owner);
				}
			}
			// prepare unstaking of collator candidate
			Self::prep_unstake(&state.id, state.stake, true)?;

			// *** No Fail beyond this point ***

			// increment rewards of collator
			Self::do_inc_collator_reward(collator, state.stake);

			// disable validator for next session if they were in the set of validators
			pallet_session::Pallet::<T>::validators()
				.into_iter()
				.enumerate()
				.find_map(|(i, id)| {
					if <T as pallet_session::Config>::ValidatorIdOf::convert(collator.clone()) ==
						Some(id)
					{
						Some(i)
					} else {
						None
					}
				})
				.map(u32::saturated_from::<usize>)
				// FIXME: Does not prevent the collator from being able to author a block in this (or potentially the next) session. See https://github.com/paritytech/substrate/issues/8004
				.map(pallet_session::Pallet::<T>::disable_index);

			// Kill storage
			BlocksAuthored::<T>::remove(collator);
			BlocksRewarded::<T>::remove(collator);
			CandidatePool::<T>::remove(collator);
			Ok(())
		}

		/// Withdraw all staked currency which was unstaked at least
		/// `StakeDuration` blocks ago.
		fn do_unlock(who: &T::AccountId) -> Result<u32, DispatchError> {
			let now = frame_system::Pallet::<T>::block_number();
			let mut unstaking = Unstaking::<T>::get(who);
			let unstaking_len = unstaking.len().saturated_into::<u32>();
			ensure!(!unstaking.is_empty(), Error::<T>::UnstakingIsEmpty);

			let mut total_unlocked: BalanceOf<T> = Zero::zero();
			let mut total_locked: BalanceOf<T> = Zero::zero();
			let mut expired = Vec::new();

			// check potential unlocks
			for (block_number, locked_balance) in unstaking.clone().into_iter() {
				if block_number <= now {
					expired.push(block_number);
					total_unlocked = total_unlocked.saturating_add(locked_balance);
				} else {
					total_locked = total_locked.saturating_add(locked_balance);
				}
			}
			for block_number in expired {
				unstaking.remove(&block_number);
			}

			// iterate balance locks to retrieve amount of locked balance
			let locks = Locks::<T>::get(who);
			total_locked = if let Some(BalanceLock { amount, .. }) =
				locks.iter().find(|l| l.id == STAKING_ID)
			{
				amount.saturating_sub(total_unlocked.into()).into()
			} else {
				// should never fail to find the lock since we checked whether unstaking is not
				// empty but let's be safe
				Zero::zero()
			};

			if total_locked.is_zero() {
				T::Currency::remove_lock(STAKING_ID, who);
				Unstaking::<T>::remove(who);
			} else {
				T::Currency::set_lock(STAKING_ID, who, total_locked, WithdrawReasons::all());
				Unstaking::<T>::insert(who, unstaking);
			}

			Ok(unstaking_len)
		}

		// /// Process the coinbase rewards for the production of a new block.
		// fn do_reward(pot: &T::AccountId, who: &T::AccountId, reward: BalanceOf<T>) {
		// 	if let Ok(_success) = T::Currency::transfer(pot, who, reward,
		// ExistenceRequirement::KeepAlive) { 		Self::deposit_event(Event::Rewarded(who.clone(),
		// reward)); 	}
		// }

		/// Checks whether a delegator can still delegate in this round, e.g.,
		/// if they have not delegated MaxDelegationsPerRound many times
		/// already in this round.
		fn get_delegation_counter(
			delegator: &T::AccountId,
		) -> Result<DelegationCounter, DispatchError> {
			let last_delegation = LastDelegation::<T>::get(delegator);
			let round = Round::<T>::get();

			// reset counter if the round advanced since last delegation
			let counter =
				if last_delegation.round < round.current { 0u32 } else { last_delegation.counter };

			ensure!(
				counter < T::MaxDelegationsPerRound::get(),
				Error::<T>::DelegationsPerRoundExceeded
			);

			Ok(DelegationCounter { round: round.current, counter: counter.saturating_add(1) })
		}

		/// Calculates the collator staking rewards for authoring `multiplier`
		/// many blocks based on the given stake.
		///
		/// Depends on the average total block reward and configurable reward
		/// rate for collators.
		// Depends on the current total issuance and staking reward
		// configuration for collators.
		pub(crate) fn calc_block_rewards_collator(
			_stake: BalanceOf<T>,
			multiplier: BalanceOf<T>,
		) -> BalanceOf<T> {
			// Note: Original Kilt-solution, keep comments for the moment
			// let total_issuance = T::Currency::total_issuance();
			// let TotalStake {
			// 	collators: total_collators,
			// 	..
			// } = TotalCollatorStake::<T>::get();
			// let staking_rate = Perquintill::from_rational(total_collators, total_issuance);

			// InflationConfig::<T>::get()
			// 	.collator
			//	.compute_reward::<T>(stake, staking_rate, multiplier)

			// Note: Due to Peaq's varying block-rewards, we need an average value and we
			// 	     do not mint tokens dynamically in dependency on demand
			let avg_block_reward = T::AvgBlockRewardProvider::get_average_for(
				T::AvgBlockRewardRecipient::get()
			);
			let reward_rate_config = RewardRateConfig::<T>::get();
			reward_rate_config.compute_collator_reward::<T>(avg_block_reward) * multiplier
		}

		/// Calculates the delegator staking rewards for `multiplier` many
		/// blocks based on the given stake.
		///
		/// Depends on the average total block reward and percentage of staking
		/// among delegators of current block author.
		// Depends on the current total issuance and staking reward
		// configuration for delegators.
		pub(crate) fn calc_block_rewards_delegator(
			stake: BalanceOf<T>,
			multiplier: BalanceOf<T>,
		) -> BalanceOf<T> {
			// Note: Original Kilt-solution, keep comments for the moment
			// let total_issuance = T::Currency::total_issuance();
			// let TotalStake {
			// 	delegators: total_delegators,
			// 	..
			// } = TotalCollatorStake::<T>::get();
			// let staking_rate = Perquintill::from_rational(total_delegators, total_issuance);

			// InflationConfig::<T>::get()
			//	.delegator
			//	.compute_reward::<T>(stake, staking_rate, multiplier)

			// Note: Due to Peaq's varying block-rewards, we need an average value and we
			// 	     do not mint tokens dynamically in dependency on demand
			let avg_block_reward = T::AvgBlockRewardProvider::get_average_for(
				T::AvgBlockRewardRecipient::get()
			);
			let reward_rate_config = RewardRateConfig::<T>::get();
			let total_stake = TotalCollatorStake::<T>::get();
			let staking_rate = Perquintill::from_rational(stake, total_stake.delegators);
			reward_rate_config.compute_delegator_reward::<T>(avg_block_reward, staking_rate) *
				multiplier
		}

		/// Increment the accumulated rewards of a collator.
		///
		/// Updates Rewarded(col) and sets BlocksRewarded(col) to equal
		/// BlocksAuthored(col).
		fn do_inc_collator_reward(acc: &T::AccountId, stake: BalanceOf<T>) {
			let count_authored = BlocksAuthored::<T>::get(acc);
			// We can already mutate thanks to extrinsics being transactional
			let count_rewarded = BlocksRewarded::<T>::mutate(acc, |rewarded| {
				let old = *rewarded;
				*rewarded = count_authored;
				old
			});
			let unclaimed_blocks = count_authored.saturating_sub(count_rewarded);
			// Note: At Peaq, we don't mint on top, we "take" it from the Pot
			let (withdrawel, _) = T::Currency::slash(
				&Self::account_id(),
				Self::calc_block_rewards_collator(stake, unclaimed_blocks.into()),
			);

			Rewards::<T>::mutate(acc, |reward| {
				*reward = reward.saturating_add(withdrawel.peek());
			});
		}

		/// Increment the accumulated rewards of a delegator by checking the
		/// number of authored blocks by the collator.
		///
		/// Updates Rewarded(del) and sets BlocksRewarded(del) to equal
		/// BlocksAuthored(col).
		fn do_inc_delegator_reward(acc: &T::AccountId, stake: BalanceOf<T>, col: &T::AccountId) {
			let count_authored = BlocksAuthored::<T>::get(col);
			// We can already mutate thanks to extrinsics being transactional
			let count_rewarded = BlocksRewarded::<T>::mutate(acc, |rewarded| {
				let old = *rewarded;
				*rewarded = count_authored;
				old
			});
			let unclaimed_blocks = count_authored.saturating_sub(count_rewarded);
			// Note: At Peaq, we don't mint on top, we "take" it from the Pot
			let (withdrawel, _) = T::Currency::slash(
				&Self::account_id(),
				Self::calc_block_rewards_delegator(stake, unclaimed_blocks.into()),
			);

			Rewards::<T>::mutate(acc, |reward| {
				*reward = reward.saturating_add(withdrawel.peek());
			});
		}

		/// Transforms the given PotId into an AccountId
		pub(crate) fn account_id() -> T::AccountId {
			T::PotId::get().into_account_truncating()
		}
	}

	impl<T> pallet_authorship::EventHandler<T::AccountId, T::BlockNumber> for Pallet<T>
	where
		T: Config + pallet_authorship::Config + pallet_session::Config,
	{
		/// Increments the reward counter of the block author by the current
		/// number of collators in the session.
		fn note_author(author: T::AccountId) {
			// should always include state except if the collator has been forcedly removed
			// via `force_remove_candidate` in the current or previous round
			if CandidatePool::<T>::get(&author).is_some() {
				// necessary to compensate for a potentially fluctuating number of collators
				// let authors = pallet_session::Pallet::<T>::validators();
				BlocksAuthored::<T>::mutate(&author, |count| {
					// *count = count.saturating_add(authors.len().
					// saturated_into::<T::BlockNumber>());
					*count = count.saturating_add(1u128.saturated_into::<T::BlockNumber>());
				});
			}

			frame_system::Pallet::<T>::register_extra_weight_unchecked(
				T::DbWeight::get().reads_writes(2, 1),
				DispatchClass::Mandatory,
			);
		}

		fn note_uncle(_author: T::AccountId, _age: T::BlockNumber) {
			// we too are not caring.
		}
	}

	impl<T: Config> pallet_session::SessionManager<T::AccountId> for Pallet<T> {
		/// 1. A new session starts.
		/// 2. In hook new_session: Read the current top n candidates from the
		///    TopCandidates and assign this set to author blocks for the next
		///    session.
		/// 3. AuRa queries the authorities from the session pallet for
		///    this session and picks authors on round-robin-basis from list of
		///    authorities.
		fn new_session(new_index: SessionIndex) -> Option<Vec<T::AccountId>> {
			log::debug!(
				"assembling new collators for new session {} at #{:?}",
				new_index,
				frame_system::Pallet::<T>::block_number(),
			);

			frame_system::Pallet::<T>::register_extra_weight_unchecked(
				T::DbWeight::get().reads(2),
				DispatchClass::Mandatory,
			);

			let collators = Pallet::<T>::selected_candidates().to_vec();
			if collators.is_empty() {
				// we never want to pass an empty set of collators. This would brick the chain.
				log::error!("💥 keeping old session because of empty collator set!");
				None
			} else {
				Some(collators)
			}
		}

		fn end_session(_end_index: SessionIndex) {
			// we too are not caring.
		}

		fn start_session(_start_index: SessionIndex) {
			// we too are not caring.
		}
	}

	impl<T: Config> ShouldEndSession<T::BlockNumber> for Pallet<T> {
		fn should_end_session(now: T::BlockNumber) -> bool {
			frame_system::Pallet::<T>::register_extra_weight_unchecked(
				T::DbWeight::get().reads(2),
				DispatchClass::Mandatory,
			);

			let mut round = Round::<T>::get();
			// always update when a new round should start
			if round.should_update(now) {
				true
			} else if ForceNewRound::<T>::get() {
				frame_system::Pallet::<T>::register_extra_weight_unchecked(
					T::DbWeight::get().writes(2),
					DispatchClass::Mandatory,
				);
				// check for forced new round
				ForceNewRound::<T>::put(false);
				round.update(now);
				Round::<T>::put(round);
				Self::deposit_event(Event::NewRound(round.first, round.current));
				true
			} else {
				false
			}
		}
	}

	impl<T: Config> EstimateNextSessionRotation<T::BlockNumber> for Pallet<T> {
		fn average_session_length() -> T::BlockNumber {
			Round::<T>::get().length
		}

		fn estimate_current_session_progress(now: T::BlockNumber) -> (Option<Permill>, Weight) {
			let round = Round::<T>::get();
			let passed_blocks = now.saturating_sub(round.first);

			(
				Some(Permill::from_rational(passed_blocks, round.length)),
				// One read for the round info, blocknumber is read free
				T::DbWeight::get().reads(1),
			)
		}

		fn estimate_next_session_rotation(
			_now: T::BlockNumber,
		) -> (Option<T::BlockNumber>, Weight) {
			let round = Round::<T>::get();

			(
				Some(round.first + round.length),
				// One read for the round info, blocknumber is read free
				T::DbWeight::get().reads(1),
			)
		}
	}

	impl<T: Config> OnUnbalanced<NegativeImbalanceOf<T>> for Pallet<T> {
		fn on_unbalanced(imbalance: NegativeImbalanceOf<T>) {
			Self::on_nonzero_unbalanced(imbalance);
		}

		fn on_nonzero_unbalanced(imbalance: NegativeImbalanceOf<T>) {
			let pot = T::PotId::get().into_account_truncating();
			// let amount = imbalance.peek();
			T::Currency::resolve_creating(&pot, imbalance);
			// Self::register_incoming_block_reward(amount);
		}
	}
}
