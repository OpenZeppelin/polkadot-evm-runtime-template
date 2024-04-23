use frame_support::{
    parameter_types,
    traits::{ConstU32, Contains, Everything, Nothing, PalletInfoAccess},
    weights::Weight,
};
use frame_system::EnsureRoot;
use pallet_xcm::XcmPassthrough;
use polkadot_parachain_primitives::primitives::Sibling;
use xcm::latest::prelude::*;
use xcm_builder::{
    AccountKey20Aliases, AllowExplicitUnpaidExecutionFrom, AllowTopLevelPaidExecutionFrom,
    DenyReserveTransferToRelayChain, DenyThenTry, EnsureXcmOrigin, FixedWeightBounds,
    FrameTransactionalProcessor, FungibleAdapter, FungiblesAdapter, IsConcrete, NativeAsset,
    NoChecking, ParentIsPreset, RelayChainAsNative, SiblingParachainAsNative,
    SiblingParachainConvertsVia, SignedAccountKey20AsNative, SovereignSignedViaLocation,
    TakeWeightCredit, TrailingSetTopicAsId, UsingComponents, WithComputedOrigin, WithUniqueTopic,
};
use xcm_executor::XcmExecutor;

use super::{
    AccountId, AllPalletsWithSystem, Assets, Balance, Balances, ParachainInfo, ParachainSystem,
    PolkadotXcm, Runtime, RuntimeCall, RuntimeEvent, RuntimeOrigin, WeightToFee, XcmpQueue,
};

parameter_types! {
    pub const RelayLocation: Location = Location::parent();
    pub const RelayNetwork: Option<NetworkId> = None;
    pub PlaceholderAccount: AccountId = PolkadotXcm::check_account();
    pub AssetsPalletLocation: Location =
        PalletInstance(<Assets as PalletInfoAccess>::index() as u8).into();
    pub RelayChainOrigin: RuntimeOrigin = cumulus_pallet_xcm::Origin::Relay.into();
    pub UniversalLocation: InteriorLocation = Parachain(ParachainInfo::parachain_id().into()).into();
}

/// `AssetId/Balancer` converter for `TrustBackedAssets`
pub type TrustBackedAssetsConvertedConcreteId =
    assets_common::TrustBackedAssetsConvertedConcreteId<AssetsPalletLocation, Balance>;

/// Type for specifying how a `Location` can be converted into an
/// `AccountId`. This is used when determining ownership of accounts for asset
/// transacting and when attempting to use XCM `Transact` in order to determine
/// the dispatch Origin.
pub type LocationToAccountId = (
    // The parent (Relay-chain) origin converts to the parent `AccountId`.
    ParentIsPreset<AccountId>,
    // Sibling parachain origins convert to AccountId via the `ParaId::into`.
    SiblingParachainConvertsVia<Sibling, AccountId>,
    // If we receive a Location of type AccountKey20, just generate a native account
    AccountKey20Aliases<RelayNetwork, AccountId>,
);

/// Means for transacting assets on this chain.
pub type LocalAssetTransactor = FungibleAdapter<
    // Use this currency:
    Balances,
    // Use this currency when it is a fungible asset matching the given location or name:
    IsConcrete<RelayLocation>,
    // Do a simple punn to convert an AccountId20 Location into a native chain account ID:
    LocationToAccountId,
    // Our chain's account ID type (we can't get away without mentioning it explicitly):
    AccountId,
    // We don't track any teleports.
    (),
>;

/// Means for transacting assets besides the native currency on this chain.
pub type LocalFungiblesTransactor = FungiblesAdapter<
    // Use this fungibles implementation:
    Assets,
    // Use this currency when it is a fungible asset matching the given location or name:
    TrustBackedAssetsConvertedConcreteId,
    // Convert an XCM MultiLocation into a local account id:
    LocationToAccountId,
    // Our chain's account ID type (we can't get away without mentioning it explicitly):
    AccountId,
    // We don't track any teleports of `Assets`.
    NoChecking,
    // We don't track any teleports of `Assets`, but a placeholder account is provided due to trait
    // bounds.
    PlaceholderAccount,
>;

/// Means for transacting assets on this chain.
pub type AssetTransactors = (LocalAssetTransactor, LocalFungiblesTransactor);

/// This is the type we use to convert an (incoming) XCM origin into a local
/// `Origin` instance, ready for dispatching a transaction with Xcm's
/// `Transact`. There is an `OriginKind` which can biases the kind of local
/// `Origin` it will become.
pub type XcmOriginToTransactDispatchOrigin = (
    // Sovereign account converter; this attempts to derive an `AccountId` from the origin location
    // using `LocationToAccountId` and then turn that into the usual `Signed` origin. Useful for
    // foreign chains who want to have a local sovereign account on this chain which they control.
    SovereignSignedViaLocation<LocationToAccountId, RuntimeOrigin>,
    // Native converter for Relay-chain (Parent) location; will convert to a `Relay` origin when
    // recognized.
    RelayChainAsNative<RelayChainOrigin, RuntimeOrigin>,
    // Native converter for sibling Parachains; will convert to a `SiblingPara` origin when
    // recognized.
    SiblingParachainAsNative<cumulus_pallet_xcm::Origin, RuntimeOrigin>,
    // Xcm Origins defined by a Multilocation of type AccountKey20 can be converted to a 20 byte-
    // account local origin
    SignedAccountKey20AsNative<RelayNetwork, RuntimeOrigin>,
    // Xcm origins can be represented natively under the Xcm pallet's Xcm origin.
    XcmPassthrough<RuntimeOrigin>,
);

parameter_types! {
    // One XCM operation is 1_000_000_000 weight - almost certainly a conservative estimate.
    pub const UnitWeightCost: Weight = Weight::from_parts(1_000_000_000, 64 * 1024);
    pub const MaxInstructions: u32 = 100;
    pub const MaxAssetsIntoHolding: u32 = 64;
}

pub struct ParentOrParentsExecutivePlurality;
impl Contains<Location> for ParentOrParentsExecutivePlurality {
    fn contains(location: &Location) -> bool {
        matches!(location.unpack(), (1, []) | (1, [Plurality { id: BodyId::Executive, .. }]))
    }
}

pub type Barrier = TrailingSetTopicAsId<
    DenyThenTry<
        DenyReserveTransferToRelayChain,
        (
            TakeWeightCredit,
            WithComputedOrigin<
                (
                    AllowTopLevelPaidExecutionFrom<Everything>,
                    AllowExplicitUnpaidExecutionFrom<ParentOrParentsExecutivePlurality>,
                    // ^^^ Parent and its exec plurality get free execution
                ),
                UniversalLocation,
                ConstU32<8>,
            >,
        ),
    >,
>;

use frame_support::traits::{Currency, OnUnbalanced};

pub struct ToAuthor<R>(sp_std::marker::PhantomData<R>);
// use frame_support::traits::fungible::{Balanced, Credit},
// impl<R> OnUnbalanced<Credit<R::AccountId, pallet_balances::Pallet<R>>> for ToAuthor<R>
// where
//     R: pallet_balances::Config + pallet_authorship::Config,
//     pallet_balances::Pallet<R> as Currency<R::AccountId>>::NegativeImbalance: From<Credit<R::AccountId, pallet_balances::Pallet<R>>>;
// {
//     fn on_nonzero_unbalanced(
//         amount: Credit<<R as frame_system::Config>::AccountId, pallet_balances::Pallet<R>>,
//     ) {
//         if let Some(author) = <pallet_authorship::Pallet<R>>::author() {
//             let _ = <pallet_balances::Pallet<R>>::resolve(&author, amount);
//         }
//     }
// }
impl<R> OnUnbalanced<<pallet_balances::Pallet<R> as Currency<R::AccountId>>::NegativeImbalance>
    for ToAuthor<R>
where
    R: pallet_balances::Config + pallet_authorship::Config,
{
    fn on_nonzero_unbalanced(
        _amount: <pallet_balances::Pallet<R> as Currency<R::AccountId>>::NegativeImbalance,
    ) { /*drop negative imbalances*/
    }
}

pub struct XcmConfig;
impl xcm_executor::Config for XcmConfig {
    type Aliasers = Nothing;
    type AssetClaims = PolkadotXcm;
    type AssetExchanger = ();
    type AssetLocker = ();
    // How to withdraw and deposit an asset.
    type AssetTransactor = AssetTransactors;
    type AssetTrap = PolkadotXcm;
    type Barrier = Barrier;
    type CallDispatcher = RuntimeCall;
    type FeeManager = ();
    type IsReserve = NativeAsset;
    type IsTeleporter = ();
    type MaxAssetsIntoHolding = MaxAssetsIntoHolding;
    type MessageExporter = ();
    type OriginConverter = XcmOriginToTransactDispatchOrigin;
    type PalletInstancesInfo = AllPalletsWithSystem;
    type ResponseHandler = PolkadotXcm;
    type RuntimeCall = RuntimeCall;
    type SafeCallFilter = Everything;
    type SubscriptionService = PolkadotXcm;
    type Trader =
        UsingComponents<WeightToFee, RelayLocation, AccountId, Balances, ToAuthor<Runtime>>;
    type TransactionalProcessor = FrameTransactionalProcessor;
    type UniversalAliases = Nothing;
    // Teleporting is disabled.
    type UniversalLocation = UniversalLocation;
    type Weigher = FixedWeightBounds<UnitWeightCost, RuntimeCall, MaxInstructions>;
    type XcmSender = XcmRouter;
}

use frame_support::{pallet_prelude::Get, traits::OriginTrait};
use sp_runtime::traits::TryConvert;

// Convert a local Origin (i.e., a signed 20 byte account Origin)  to a Multilocation
pub struct SignedToAccountId20<Origin, AccountId, Network>(
    sp_std::marker::PhantomData<(Origin, AccountId, Network)>,
);
impl<Origin: OriginTrait + Clone, AccountId: Into<[u8; 20]>, Network: Get<Option<NetworkId>>>
    TryConvert<Origin, Location> for SignedToAccountId20<Origin, AccountId, Network>
where
    Origin::PalletsOrigin: From<frame_system::RawOrigin<AccountId>>
        + TryInto<frame_system::RawOrigin<AccountId>, Error = Origin::PalletsOrigin>,
{
    fn try_convert(o: Origin) -> Result<Location, Origin> {
        o.try_with_caller(|caller| match caller.try_into() {
            Ok(frame_system::RawOrigin::Signed(who)) =>
                Ok(AccountKey20 { key: who.into(), network: Network::get() }.into()),
            Ok(other) => Err(other.into()),
            Err(other) => Err(other),
        })
    }
}

// Converts a Signed Local Origin into a Location
pub type LocalOriginToLocation = SignedToAccountId20<RuntimeOrigin, AccountId, RelayNetwork>;

/// The means for routing XCM messages which are not for local execution into
/// the right message queues.
pub type XcmRouter = WithUniqueTopic<(
    // Two routers - use UMP to communicate with the relay chain:
    cumulus_primitives_utility::ParentAsUmp<ParachainSystem, (), ()>,
    // ..and XCMP to communicate with the sibling chains.
    XcmpQueue,
)>;

parameter_types! {
    pub const MaxLockers: u32 = 8;
    pub const MaxRemoteLockConsumers: u32 = 0;
}

impl pallet_xcm::Config for Runtime {
    type AdminOrigin = EnsureRoot<AccountId>;
    // ^ Override for AdvertisedXcmVersion default
    type AdvertisedXcmVersion = pallet_xcm::CurrentXcmVersion;
    type Currency = Balances;
    type CurrencyMatcher = ();
    type ExecuteXcmOrigin = EnsureXcmOrigin<RuntimeOrigin, LocalOriginToLocation>;
    type MaxLockers = MaxLockers;
    type MaxRemoteLockConsumers = MaxLockers;
    type RemoteLockConsumerIdentifier = ();
    type RuntimeCall = RuntimeCall;
    type RuntimeEvent = RuntimeEvent;
    type RuntimeOrigin = RuntimeOrigin;
    type SendXcmOrigin = EnsureXcmOrigin<RuntimeOrigin, LocalOriginToLocation>;
    type SovereignAccountOf = LocationToAccountId;
    type TrustedLockers = ();
    type UniversalLocation = UniversalLocation;
    type Weigher = FixedWeightBounds<UnitWeightCost, RuntimeCall, MaxInstructions>;
    type WeightInfo = pallet_xcm::TestWeightInfo;
    type XcmExecuteFilter = Nothing;
    // ^ Disable dispatchable execute on the XCM pallet.
    // Needs to be `Everything` for local testing.
    type XcmExecutor = XcmExecutor<XcmConfig>;
    type XcmReserveTransferFilter = Nothing;
    type XcmRouter = XcmRouter;
    type XcmTeleportFilter = Everything;

    const VERSION_DISCOVERY_QUEUE_SIZE: u32 = 100;
}

impl cumulus_pallet_xcm::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type XcmExecutor = XcmExecutor<XcmConfig>;
}
