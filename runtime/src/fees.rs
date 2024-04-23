use frame_support::traits::fungible::{Balanced, Credit};
/// Transaction fee handling logic
use frame_support::traits::{tokens::imbalance::ResolveTo, Imbalance, OnUnbalanced};

/// Logic for the author to get a portion of fees.
/// TODO: move to XCM Config above where it is used
pub struct ToAuthor<R>(sp_std::marker::PhantomData<R>);
impl<R> OnUnbalanced<Credit<R::AccountId, pallet_balances::Pallet<R>>> for ToAuthor<R>
where
    R: pallet_balances::Config + pallet_authorship::Config,
{
    fn on_nonzero_unbalanced(
        amount: Credit<<R as frame_system::Config>::AccountId, pallet_balances::Pallet<R>>,
    ) {
        if let Some(author) = <pallet_authorship::Pallet<R>>::author() {
            let _ = <pallet_balances::Pallet<R>>::resolve(&author, amount);
        }
    }
}
