use anchor_lang::prelude::*;

#[error_code]
pub enum BurrpadError {
    #[msg("Only the program's upgrade key may initialise")]
    NotUpgradeAuthority,
    #[msg("Only the administrator may do this")]
    NotAdmin,
    #[msg("Only the proposed administrator may accept")]
    NotPendingAdmin,
    #[msg("Fee setting outside the hard limits, or not lowest <= first <= highest")]
    FeeSettingOutOfRange,
    #[msg("Platform fraction above the hard limit")]
    PlatformFractionTooHigh,
    #[msg("The empty address cannot be a receiver")]
    EmptyAddress,
    #[msg("New launches are switched off")]
    LaunchesPaused,
    #[msg("Fee rate outside the allowed range")]
    FeeOutOfRange,
    #[msg("Hook shares must add up to the whole")]
    SharesDoNotAddUp,
    #[msg("A chosen hook is missing its receiver")]
    MissingReceiver,
    #[msg("Name, ticker or link too long")]
    TextTooLong,
    #[msg("The coin pump.fun created is not what was asked for")]
    CurveMismatch,
    #[msg("The coin's mint is not the plain kind expected")]
    UnexpectedMint,
    #[msg("A pump.fun account is not the one it should be")]
    WrongPumpAccount,
    #[msg("A pump.fun account could not be read")]
    UnreadablePumpAccount,
    #[msg("This receiver is not the one in the coin's record")]
    WrongReceiver,
    #[msg("Arithmetic overflow")]
    Overflow,
    #[msg("The vault holds less than the ledger says")]
    VaultShort,
    #[msg("This hook was not chosen for this coin")]
    HookNotChosen,
    #[msg("Nothing to spend")]
    NothingToBurn,
    #[msg("Too soon since the last buy back")]
    BurnTooSoon,
    #[msg("The buy returned fewer coins than the pool's own figures allow")]
    BurnBadFill,
    #[msg("The buy took more than was set aside for it")]
    BurnOverspent,
    #[msg("The coin has not graduated to a pool, or has already left the curve")]
    WrongVenue,
    #[msg("Only the rewards wallet in the settings may take a holder pot")]
    NotRewardsWallet,
    #[msg("Too soon since this coin's last holder release")]
    HolderReleaseTooSoon,
    #[msg("A holder release must take more than nothing")]
    NothingToRelease,
    #[msg("The snapshot's slot or fingerprint is not acceptable")]
    BadSnapshot,
    #[msg("A receiver cannot be one of the coin's own program-controlled addresses")]
    ReceiverNotAllowed,
    #[msg("The platform's terms changed since this launch was prepared")]
    PlatformTermsChanged,
    #[msg("No new rewards wallet has been proposed")]
    NoRewardsWalletProposed,
    #[msg("The proposed rewards wallet's wait is not over")]
    RewardsWalletWaitNotOver,
    #[msg("Holder releases are switched off")]
    HolderReleasesPaused,
    #[msg("The holder pot holds less than the amount asked for")]
    ReleaseAboveThePot,
}
