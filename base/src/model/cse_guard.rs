//! The CSE member guard flag and the only scope that may suspend it.
//!
//! The guard rejects a user write into a position covered by a CSE array
//! rectangle, including one whose member cell a structural edit has already
//! dropped while the anchor still owns and refills the rectangle.
//!
//! A structural rebuild — a cell, row or column move — legitimately writes
//! into such a rectangle: it rewrites the anchor and then the placeholders of
//! the rectangle the anchor has just re-declared. It has to suspend the guard
//! to do that.
//!
//! Suspension is scoped, and this module is why it cannot be anything else.
//! The flag is a private field of [`CseMemberGuard`], so no code outside this
//! file can write it — a hand-rolled set-then-reset pair, which an early `?`
//! between the two halves would leak, does not compile. The only two
//! operations exported are [`CseMemberGuard::is_suspended`], for the guard
//! check itself, and [`Model::with_cse_guard_suspended`], which restores the
//! previous value on every exit path.

use crate::model::Model;

/// Whether the CSE member guard is currently suspended.
///
/// Not a public type: constructing one is harmless (it starts unsuspended),
/// but flipping it is not, and only this module can.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CseMemberGuard {
    /// Private on purpose. See the module docs.
    suspended: bool,
}

impl CseMemberGuard {
    pub(crate) fn is_suspended(self) -> bool {
        self.suspended
    }
}

impl Model<'_> {
    /// Runs `f` with the CSE member guard suspended, restoring the previous
    /// state whether `f` returns, propagates with `?`, or errors.
    ///
    /// Nesting is fine: the previous value is saved rather than assumed, so an
    /// inner scope cannot un-suspend an outer one on its way out.
    ///
    /// That rebuild paths *reach for* this scope is still convention, checked
    /// by the grep-gate `unchecked_rebuild_paths_suspend_the_cse_member_guard`.
    /// That they cannot suspend the guard any other way is not: the flag is
    /// private to this module.
    pub(crate) fn with_cse_guard_suspended<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        let previous = std::mem::replace(&mut self.cse_member_guard.suspended, true);
        let result = f(self);
        self.cse_member_guard.suspended = previous;
        result
    }
}
