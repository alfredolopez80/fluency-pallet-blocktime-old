#![cfg_attr(not(feature = "std"), no_std)]

use frame_support::{decl_module, decl_storage, traits::Get};

use std::{result, cmp};
use inherents::{ProvideInherent, InherentData, InherentIdentifier};
#[cfg(feature = "std")]
use frame_support::debug;
use frame_support::{
	Parameter,
	traits::{Time, UnixTime},
	weights::{DispatchClass, Weight},
};
use runtime::{
	RuntimeString,
	traits::{
		AtLeast32Bit, Zero, SaturatedConversion, Scale
	}
};
use frame_system::ensure_none;
use pallet_timestamp::{
	InherentError, INHERENT_IDENTIFIER, InherentType,
	OnTimestampSet,
};


pub trait WeightInfo {
	fn set() -> Weight;
	fn on_finalize() -> Weight;
}

#[pallet_blocktime::config]
pub trait Config: frame_system::Config {
    type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;
    type TimeProvider: UnixTime;
}

/// The module configuration trait
pub trait Trait: frame_system::Trait {
	/// Type used for expressing timestamp.
	type Moment: Parameter + Default + AtLeast32Bit
		+ Scale<Self::BlockNumber, Output = Self::Moment> + Copy;

	/// Something which can be notified when the timestamp is set. Set this to `()` if not needed.
	type OnTimestampSet: OnTimestampSet<Self::Moment>;

	/// The minimum period between blocks. Beware that this is different to the *expected* period
	/// that the block production apparatus provides. Your chosen consensus system will generally
	/// work with this to determine a sensible block time. e.g. For Aura, it will be double this
	/// period on default settings.
	type MinimumPeriod: Get<Self::Moment>;

	/// Weight information for extrinsics in this pallet.
	type WeightInfo: WeightInfo;
}

decl_module! {
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {
		/// The minimum period between blocks. Beware that this is different to the *expected* period
		/// that the block production apparatus provides. Your chosen consensus system will generally
		/// work with this to determine a sensible block time. e.g. For Aura, it will be double this
		/// period on default settings.
		const MinimumPeriod: T::Moment = T::MinimumPeriod::get();

		/// Set the current time.
		///
		/// This call should be invoked exactly once per block. It will panic at the finalization
		/// phase, if this call hasn't been invoked by that time.
		///
		/// The timestamp should be greater than the previous one by the amount specified by
		/// `MinimumPeriod`.
		///
		/// The dispatch origin for this call must be `Inherent`.
		///
		/// # <weight>
		/// - `O(T)` where `T` complexity of `on_timestamp_set`
		/// - 1 storage read and 1 storage mutation (codec `O(1)`). (because of `DidUpdate::take` in `on_finalize`)
		/// - 1 event handler `on_timestamp_set` `O(T)`.
		/// # </weight>
		#[weight = (
			T::WeightInfo::set(),
			DispatchClass::Mandatory
		)]
		fn set(origin, #[compact] now: T::Moment) {
			ensure_none(origin)?;
			assert!(!<Self as Store>::DidUpdate::exists(), "Timestamp must be updated only once in the block");
			let prev = Self::now();
			assert!(
				prev.is_zero() || now >= prev + T::MinimumPeriod::get(),
				"Timestamp must increment by at least <MinimumPeriod> between sequential blocks"
			);
			<Self as Store>::Now::put(now);
			<Self as Store>::DidUpdate::put(true);

			<T::OnTimestampSet as OnTimestampSet<_>>::on_timestamp_set(now);
		}

		/// dummy `on_initialize` to return the weight used in `on_finalize`.
		fn on_initialize() -> Weight {
			// weight of `on_finalize`
			T::WeightInfo::on_finalize()
		}

		/// # <weight>
		/// - `O(1)`
		/// - 1 storage deletion (codec `O(1)`).
		/// # </weight>
		fn on_finalize() {
			assert!(<Self as Store>::DidUpdate::take(), "Timestamp must be updated once in the block");
		}
	}
}

decl_storage! {
	trait Store for Module<T: Trait> as Timestamp {
		/// Current time for the current block.
		pub Now get(fn now) build(|_| 0u32.into()): T::Moment;

		/// Did the timestamp get updated in this block?
		DidUpdate: bool;
	}
}

impl<T: Trait> Module<T> {
	/// Get the current time for the current block.
	///
	/// NOTE: if this function is called prior to setting the timestamp,
	/// it will return the timestamp of the previous block.
	pub fn get() -> T::Moment {
		Self::now()
	}

	/// Set the timestamp to something in particular. Only used for tests.
	#[cfg(feature = "std")]
	pub fn set_timestamp(now: T::Moment) {
		<Self as Store>::Now::put(now);
	}
}

fn extract_inherent_data(data: &InherentData) -> Result<InherentType, RuntimeString> {
	data.get_data::<InherentType>(&INHERENT_IDENTIFIER)
		.map_err(|_| RuntimeString::from("Invalid timestamp inherent data encoding."))?
		.ok_or_else(|| "Timestamp inherent data is not provided.".into())
}

impl<T: Trait> ProvideInherent for Module<T> {
	type Call = Call<T>;
	type Error = InherentError;
	const INHERENT_IDENTIFIER: InherentIdentifier = INHERENT_IDENTIFIER;

	fn create_inherent(data: &InherentData) -> Option<Self::Call> {
		let data: T::Moment = extract_inherent_data(data)
			.expect("Gets and decodes timestamp inherent data")
			.saturated_into();

		let next_time = cmp::max(data, Self::now() + T::MinimumPeriod::get());
		Some(Call::set(next_time.into()))
	}

	fn check_inherent(call: &Self::Call, data: &InherentData) -> result::Result<(), Self::Error> {
		const MAX_TIMESTAMP_DRIFT_MILLIS: u64 = 30 * 1000;

		let t: u64 = match call {
			Call::set(ref t) => t.clone().saturated_into::<u64>(),
			_ => return Ok(()),
		};

		let data = extract_inherent_data(data).map_err(|e| InherentError::Other(e))?;

		let minimum = (Self::now() + T::MinimumPeriod::get()).saturated_into::<u64>();
		if t > data + MAX_TIMESTAMP_DRIFT_MILLIS {
			Err(InherentError::Other("Timestamp too far in future to accept".into()))
		} else if t < minimum {
			Err(InherentError::ValidAtTimestamp(minimum))
		} else {
			Ok(())
		}
	}
}

impl<T: Trait> Time for Module<T> {
	type Moment = T::Moment;

	/// Before the first set of now with inherent the value returned is zero.
	fn now() -> Self::Moment {
		Self::now()
	}
}

/// Before the timestamp inherent is applied, it returns the time of previous block.
///
/// On genesis the time returned is not valid.
impl<T: Trait> UnixTime for Module<T> {
	fn now() -> core::time::Duration {
		// now is duration since unix epoch in millisecond as documented in
		// `sp_timestamp::InherentDataProvider`.
		let now = Self::now();
		sp_std::if_std! {
			if now == T::Moment::zero() {
				debug::error!(
					"`pallet_timestamp::UnixTime::now` is called at genesis, invalid value returned: 0"
				);
			}
		}
		core::time::Duration::from_millis(now.saturated_into::<u64>())
	}
}
