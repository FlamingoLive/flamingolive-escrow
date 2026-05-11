use anchor_lang::prelude::*;

#[error_code]
pub enum ErrorCode {
    #[msg("Program is currently paused by the circuit breaker.")]
    ProgramPaused,

    #[msg("Circuit breaker tripped: volume threshold exceeded. Admin must unpause.")]
    CircuitBreakerTripped,

    #[msg("The 24-hour dispute window has not yet elapsed. Cannot auto-release funds.")]
    InDisputeWindow,

    #[msg("Arithmetic overflow or underflow.")]
    MathOverflow,

    #[msg("Invalid order code. Does not match this escrow account.")]
    InvalidOrder,

    #[msg("Invalid status for this operation.")]
    InvalidStatus,

    #[msg("Token account owner does not match expected key.")]
    InvalidOwner,

    #[msg("Tracking ID must be between 8 and 50 characters.")]
    InvalidTrackingId,

    #[msg("Tracking ID does not match the ID registered at shipping confirmation.")]
    TrackingIdMismatch,

    #[msg("Dispute window has expired. Cannot raise a dispute after 24 hours.")]
    DisputeWindowExpired,

    #[msg("A dispute is in progress. Only the judge can resolve this escrow.")]
    DisputeInProgress,

    #[msg("Invalid carrier code. Accepted: 0=DHL, 1=Aramex, 2=FedEx, 3=Sendbox.")]
    InvalidCarrierCode,

    #[msg("Insufficient token balance for this operation.")]
    InsufficientFunds,

    #[msg("Amount must be greater than zero.")]
    InvalidAmount,

    #[msg("Invalid config parameter. Values must be greater than zero.")]
    InvalidConfig,

    #[msg("Invalid account provided.")]
    InvalidAccount,

    #[msg("Token account is frozen.")]
    AccountFrozen,

    #[msg("Invalid mint for this operation.")]
    InvalidMint,
}
