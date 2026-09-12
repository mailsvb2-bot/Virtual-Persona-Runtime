use vpr_policy::{AuthorityScope, DataClass};

use crate::cancellation::TurnCancellation;

/// Internal linearized capability for one immediate provider operation.
///
/// It is never exposed to callers; public provider execution methods acquire and consume it
/// inside one runtime call, eliminating delayed or repeated permit use.
#[derive(Debug)]
pub(crate) struct ProviderExecutionPermit {
    pub(crate) cancellation: TurnCancellation,
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderExecutionContext<'a> {
    pub(crate) required_scope: &'a AuthorityScope,
    pub(crate) data_class: DataClass,
}

impl<'a> ProviderExecutionContext<'a> {
    #[must_use]
    pub const fn new(required_scope: &'a AuthorityScope, data_class: DataClass) -> Self {
        Self {
            required_scope,
            data_class,
        }
    }
}
