//! Every account of every instruction, swapped in turn for the most
//! plausible wrong one. Each swap must be refused by Burr itself, before
//! pump.fun's programs are reached.
//!
//! The label of each swap is the name of the account in the program, so the
//! protection list (`docs/protections.md`) can say which swap holds which check.

use crate::bench::{logs, refused_by_burr, TxResult};
use crate::kit::{self, LaunchPlan, World};
use crate::pump::{self, ata, pda, PUMP, PUMP_AMM, PUMP_FEES, TOKEN, TOKEN_2022, ZEC};
use anchor_lang::{InstructionData, ToAccountMetas};
use burr::state::Payee;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;

/// The stage: two coins on the curve (`a`, `b`), two graduated (`c`, `d`),
/// all with every hook, fee collected, a staker, a king leader whose round is
/// over; a thief with accounts of the right kinds; and a program that accepts
/// any call and does nothing.
struct Stage {
    w: World,
    a: Pubkey,
    b: Pubkey,
    c: Pubkey,
    d: Pubkey,
    user: Keypair,
    thief: Keypair,
    nothing: Pubkey,
    fake: Pubkey,
}

fn stage() -> Stage {
    let mut w = World::new();
    let plan = LaunchPlan { king_round_secs: 600, ..Default::default() };
    let user = w.trader(10_000 * ZEC1);
    let thief = w.trader(ZEC1);
    let mut coins = vec![];
    for graduated in [false, false, true, true] {
        let m = w.launch(&plan);
        assert!(w.buy(&user, &m, ZEC1 / 4).is_ok());
        assert!(w.buy(&thief, &m, ZEC1 / 100).is_ok());
        assert!(w.stake(&user, &m, 1_000).is_ok());
        if graduated {
            w.graduate(&m);
            assert!(w.pool_buy(&user, &m, ZEC1).is_ok());
            assert!(w.king_buy_pool(&user, &m, ZEC1 / 10).is_ok());
            assert!(w.collect_pool(&m).is_ok());
        } else {
            assert!(w.king_buy_curve(&user, &m, ZEC1 / 10).is_ok());
        }
        assert!(w.collect_curve(&m).is_ok());
        w.b.give_zec(&kit::authority(&m), 50 * ZEC1);
        assert!(w.split(&m).is_ok());
        coins.push(m);
    }
    // Past the burn's waiting time and the king round. The first burn call on
    // each coin records the moved price; a second, a minute later, can buy.
    w.b.wait(700);
    for (i, m) in coins.iter().enumerate() {
        let r = if i < 2 { w.burn_curve(m) } else { w.burn_pool(m) };
        assert!(r.is_ok(), "{}", logs(&r));
    }
    w.b.wait(60);
    let nothing = w.b.do_nothing_program();
    Stage { w, a: coins[0], b: coins[1], c: coins[2], d: coins[3], user, thief, nothing, fake: Pubkey::new_unique() }
}

type Swap<'x, A> = (&'static str, Box<dyn Fn(&mut A) + 'x>);

fn sw<'x, A>(label: &'static str, f: impl Fn(&mut A) + 'x) -> Swap<'x, A> {
    (label, Box::new(f))
}

/// Sends the instruction once per swap and checks each is refused by Burr.
/// Then sends it unchanged and returns that result, so each test also shows
/// the right accounts are accepted.
fn each_refused<A: ToAccountMetas + Clone>(
    w: &mut World,
    signer: &Keypair,
    extra: &[&Keypair],
    good: A,
    data: impl InstructionData,
    swaps: Vec<Swap<A>>,
) -> TxResult {
    let data = data.data();
    for (label, change) in swaps {
        let mut acc = good.clone();
        change(&mut acc);
        let i = Instruction { program_id: kit::BURR, accounts: acc.to_account_metas(None), data: data.clone() };
        let r = w.b.send_as(Some(signer), &[i], extra);
        assert!(r.is_err(), "swapping {label} was accepted");
        assert!(refused_by_burr(&r), "swapping {label} was refused only by pump.fun, not by Burr\n{}", logs(&r));
    }
    let i = Instruction { program_id: kit::BURR, accounts: good.to_account_metas(None), data };
    w.b.send_as(Some(signer), &[i], extra)
}

#[test]
fn launch() {
    let Stage { mut w, a, thief, nothing, fake, .. } = stage();
    let payer = w.b.payer.insecure_clone();
    let mint = Keypair::new();
    let good = w.launch_accounts(&payer.pubkey(), &mint.pubkey());
    let plan = LaunchPlan::default();
    let data = burr::instruction::Launch {
        args: burr::LaunchArgs {
            name: plan.name.clone(),
            symbol: "TST".into(),
            uri: "https://e.co/t".into(),
            fee_bps: plan.fee_bps,
            shares: plan.shares,
            creator_receiver: plan.creator_receiver,
            give_receiver: plan.give_receiver,
            king_round_secs: plan.king_round_secs,
        },
    };
    let thief_zec = ata(&thief.pubkey(), &ZEC, &TOKEN);
    type A = burr::accounts::Launch;
    let swaps: Vec<Swap<A>> = vec![
        sw("config", |x: &mut A| x.config = fake),
        sw("coin", |x: &mut A| x.coin = kit::coin(&a)),
        sw("ledger", |x: &mut A| x.ledger = kit::ledger(&a)),
        sw("authority", |x: &mut A| x.authority = kit::authority(&a)),
        sw("vault", |x: &mut A| x.vault = thief_zec),
        sw("zec_mint", |x: &mut A| x.zec_mint = a),
        sw("pump_mint_authority", |x: &mut A| x.pump_mint_authority = fake),
        sw("bonding_curve", |x: &mut A| x.bonding_curve = pump::curve(&Pubkey::new_unique())),
        sw("associated_bonding_curve", |x: &mut A| x.associated_bonding_curve = fake),
        sw("pump_global", |x: &mut A| x.pump_global = pump::amm_global_config()),
        sw("mayhem_program", |x: &mut A| x.mayhem_program = nothing),
        sw("mayhem_global_params", |x: &mut A| x.mayhem_global_params = fake),
        sw("mayhem_sol_vault", |x: &mut A| x.mayhem_sol_vault = fake),
        sw("mayhem_state", |x: &mut A| x.mayhem_state = fake),
        sw("mayhem_token_vault", |x: &mut A| x.mayhem_token_vault = fake),
        sw("pump_event_authority", |x: &mut A| x.pump_event_authority = pump::amm_event_authority()),
        sw("associated_quote_bonding_curve", |x: &mut A| x.associated_quote_bonding_curve = thief_zec),
        sw("quote_control", |x: &mut A| x.quote_control = pump::global()),
        sw("pump_program", |x: &mut A| x.pump_program = nothing),
        sw("token_program", |x: &mut A| x.token_program = nothing),
        sw("token_2022_program", |x: &mut A| x.token_2022_program = nothing),
        sw("associated_token_program", |x: &mut A| x.associated_token_program = nothing),
        sw("system_program", |x: &mut A| x.system_program = nothing),
    ];
    let r = each_refused(&mut w, &payer, &[&mint], good, data, swaps);
    assert!(r.is_ok(), "{}", logs(&r));
}

#[test]
fn split() {
    let Stage { mut w, a, b, thief, nothing, .. } = stage();
    w.b.give_zec(&kit::authority(&a), 1_000);
    let payer = w.b.payer.insecure_clone();
    let good = burr::accounts::SplitOnly {
        mint: a,
        coin: kit::coin(&a),
        ledger: kit::ledger(&a),
        authority: kit::authority(&a),
        vault: kit::vault(&a),
        zec_mint: ZEC,
        token_program: TOKEN,
    };
    let thief_zec = ata(&thief.pubkey(), &ZEC, &TOKEN);
    type A = burr::accounts::SplitOnly;
    let swaps: Vec<Swap<A>> = vec![
        sw("coin", |x: &mut A| x.coin = kit::coin(&b)),
        sw("ledger", |x: &mut A| x.ledger = kit::ledger(&b)),
        sw("authority", |x: &mut A| x.authority = kit::authority(&b)),
        sw("vault", |x: &mut A| x.vault = thief_zec),
        sw("vault (another coin's)", |x: &mut A| x.vault = kit::vault(&b)),
        sw("zec_mint", |x: &mut A| x.zec_mint = a),
        sw("token_program", |x: &mut A| x.token_program = nothing),
    ];
    let r = each_refused(&mut w, &payer, &[], good, burr::instruction::Split {}, swaps);
    assert!(r.is_ok(), "{}", logs(&r));
}

#[test]
fn collect_curve() {
    let Stage { mut w, a, b, user, thief, nothing, fake, .. } = stage();
    assert!(w.buy(&user, &a, ZEC1).is_ok());
    let payer = w.b.payer.insecure_clone();
    let good = w.collect_curve_accounts(&payer.pubkey(), &a);
    let thief_zec = ata(&thief.pubkey(), &ZEC, &TOKEN);
    type A = burr::accounts::CollectCurve;
    let swaps: Vec<Swap<A>> = vec![
        sw("coin", |x: &mut A| x.coin = kit::coin(&b)),
        sw("ledger", |x: &mut A| x.ledger = kit::ledger(&b)),
        sw("authority", |x: &mut A| x.authority = kit::authority(&b)),
        sw("vault", |x: &mut A| x.vault = thief_zec),
        sw("zec_mint", |x: &mut A| x.zec_mint = a),
        sw("pump_global", |x: &mut A| x.pump_global = pump::amm_global_config()),
        sw("bonding_curve", |x: &mut A| x.bonding_curve = pump::curve(&b)),
        sw("associated_quote_bonding_curve", |x: &mut A| x.associated_quote_bonding_curve = thief_zec),
        sw("creator_vault", |x: &mut A| x.creator_vault = pump::creator_vault(&kit::authority(&b))),
        sw("creator_vault_ata", |x: &mut A| x.creator_vault_ata = thief_zec),
        sw("pump_event_authority", |x: &mut A| x.pump_event_authority = fake),
        sw("pump_program", |x: &mut A| x.pump_program = nothing),
        sw("token_program", |x: &mut A| x.token_program = nothing),
        sw("associated_token_program", |x: &mut A| x.associated_token_program = nothing),
        sw("system_program", |x: &mut A| x.system_program = nothing),
    ];
    let before = w.ledger(&a).accounted;
    let r = each_refused(&mut w, &payer, &[], good, burr::instruction::CollectCurve {}, swaps);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(w.ledger(&a).accounted > before);
}

#[test]
fn collect_pool() {
    let Stage { mut w, c, d, user, thief, nothing, fake, .. } = stage();
    assert!(w.pool_buy(&user, &c, ZEC1).is_ok());
    let payer = w.b.payer.insecure_clone();
    let good = w.collect_pool_accounts(&payer.pubkey(), &c);
    let thief_zec = ata(&thief.pubkey(), &ZEC, &TOKEN);
    type A = burr::accounts::CollectPool;
    let swaps: Vec<Swap<A>> = vec![
        sw("coin", |x: &mut A| x.coin = kit::coin(&d)),
        sw("ledger", |x: &mut A| x.ledger = kit::ledger(&d)),
        sw("authority", |x: &mut A| x.authority = kit::authority(&d)),
        sw("vault", |x: &mut A| x.vault = thief_zec),
        sw("zec_mint", |x: &mut A| x.zec_mint = c),
        sw("amm_global_config", |x: &mut A| x.amm_global_config = pump::global()),
        sw("pool_authority", |x: &mut A| x.pool_authority = pump::pool_authority(&d)),
        sw("pool", |x: &mut A| x.pool = pump::pool(&d)),
        sw("pool_quote_token_account", |x: &mut A| x.pool_quote_token_account = ata(&pump::pool(&d), &ZEC, &TOKEN)),
        sw("amm_creator_vault_authority", |x: &mut A| {
            x.amm_creator_vault_authority = pda(&[b"creator_vault", kit::authority(&d).as_ref()], &PUMP_AMM)
        }),
        sw("amm_creator_vault_ata", |x: &mut A| x.amm_creator_vault_ata = thief_zec),
        sw("amm_event_authority", |x: &mut A| x.amm_event_authority = fake),
        sw("amm_program", |x: &mut A| x.amm_program = nothing),
        sw("amm_program (pump.fun's other program)", |x: &mut A| x.amm_program = PUMP),
        sw("token_program", |x: &mut A| x.token_program = nothing),
        sw("associated_token_program", |x: &mut A| x.associated_token_program = nothing),
        sw("system_program", |x: &mut A| x.system_program = nothing),
    ];
    let before = w.ledger(&c).accounted;
    let r = each_refused(&mut w, &payer, &[], good, burr::instruction::CollectPool {}, swaps);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(w.ledger(&c).accounted > before);
}

#[test]
fn pay_out() {
    let Stage { mut w, a, b, thief, nothing, .. } = stage();
    let payer = w.b.payer.insecure_clone();
    let receiver = w.coin(&a).creator_receiver;
    let good = w.pay_out_accounts(&payer.pubkey(), &a, &receiver);
    let thief_zec = ata(&thief.pubkey(), &ZEC, &TOKEN);
    let thief_key = thief.pubkey();
    type A = burr::accounts::PayOut;
    let swaps: Vec<Swap<A>> = vec![
        sw("coin", |x: &mut A| x.coin = kit::coin(&b)),
        sw("ledger", |x: &mut A| x.ledger = kit::ledger(&b)),
        sw("authority", |x: &mut A| x.authority = kit::authority(&b)),
        sw("vault", |x: &mut A| x.vault = kit::vault(&b)),
        sw("zec_mint", |x: &mut A| x.zec_mint = a),
        sw("receiver", |x: &mut A| {
            x.receiver = thief_key;
            x.receiver_zec = thief_zec;
        }),
        sw("receiver_zec", |x: &mut A| x.receiver_zec = thief_zec),
        sw("token_program", |x: &mut A| x.token_program = nothing),
        sw("associated_token_program", |x: &mut A| x.associated_token_program = nothing),
        sw("system_program", |x: &mut A| x.system_program = nothing),
    ];
    let pot = w.ledger(&a).pots[1];
    assert!(pot > 0);
    let r = each_refused(&mut w, &payer, &[], good, burr::instruction::PayOut { payee: Payee::Creator }, swaps);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.zec_of(&receiver), pot);
    assert_eq!(w.b.token_balance(&thief_zec), ZEC1 - ZEC1 / 100 * 4);
}

#[test]
fn burn_curve() {
    let Stage { mut w, a, b, thief, nothing, fake, .. } = stage();
    let payer = w.b.payer.insecure_clone();
    let good = w.burn_curve_accounts(&payer.pubkey(), &a);
    let thief_zec = ata(&thief.pubkey(), &ZEC, &TOKEN);
    let thief_coin = ata(&thief.pubkey(), &a, &TOKEN_2022);
    type A = burr::accounts::BurnCurve;
    let swaps: Vec<Swap<A>> = vec![
        sw("mint", |x: &mut A| x.mint = b),
        sw("coin", |x: &mut A| x.coin = kit::coin(&b)),
        sw("ledger", |x: &mut A| x.ledger = kit::ledger(&b)),
        sw("authority", |x: &mut A| x.authority = kit::authority(&b)),
        sw("vault", |x: &mut A| x.vault = kit::vault(&b)),
        sw("zec_mint", |x: &mut A| x.zec_mint = b),
        sw("burner", |x: &mut A| x.burner = kit::burner(&b)),
        sw("burner_zec", |x: &mut A| x.burner_zec = thief_zec),
        sw("burner_coin", |x: &mut A| x.burner_coin = thief_coin),
        sw("pump_global", |x: &mut A| x.pump_global = pump::amm_global_config()),
        sw("bonding_curve", |x: &mut A| x.bonding_curve = pump::curve(&b)),
        sw("associated_base_bonding_curve", |x: &mut A| x.associated_base_bonding_curve = thief_coin),
        sw("associated_quote_bonding_curve", |x: &mut A| x.associated_quote_bonding_curve = thief_zec),
        sw("user_volume", |x: &mut A| x.user_volume = pump::user_volume(&kit::burner(&b))),
        sw("fee_config", |x: &mut A| x.fee_config = pda(&[b"fee_config", PUMP_AMM.as_ref()], &PUMP_FEES)),
        sw("buyback_fee_recipient", |x: &mut A| x.buyback_fee_recipient = thief_zec),
        sw("pump_event_authority", |x: &mut A| x.pump_event_authority = fake),
        sw("pump_program", |x: &mut A| x.pump_program = nothing),
        sw("token_program", |x: &mut A| x.token_program = nothing),
        sw("token_2022_program", |x: &mut A| x.token_2022_program = nothing),
        sw("associated_token_program", |x: &mut A| x.associated_token_program = nothing),
        sw("system_program", |x: &mut A| x.system_program = nothing),
    ];
    let pot = w.ledger(&a).pots[3];
    let r = each_refused(&mut w, &payer, &[], good, burr::instruction::BurnCurve {}, swaps);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(w.ledger(&a).pots[3] < pot, "with the right accounts the burn buys");
}

#[test]
fn burn_pool() {
    let Stage { mut w, c, d, thief, nothing, fake, .. } = stage();
    let payer = w.b.payer.insecure_clone();
    let good = w.burn_pool_accounts(&payer.pubkey(), &c);
    let thief_zec = ata(&thief.pubkey(), &ZEC, &TOKEN);
    let thief_coin = ata(&thief.pubkey(), &c, &TOKEN_2022);
    type A = burr::accounts::BurnPool;
    let swaps: Vec<Swap<A>> = vec![
        sw("mint", |x: &mut A| x.mint = d),
        sw("coin", |x: &mut A| x.coin = kit::coin(&d)),
        sw("ledger", |x: &mut A| x.ledger = kit::ledger(&d)),
        sw("authority", |x: &mut A| x.authority = kit::authority(&d)),
        sw("vault", |x: &mut A| x.vault = kit::vault(&d)),
        sw("zec_mint", |x: &mut A| x.zec_mint = d),
        sw("burner", |x: &mut A| x.burner = kit::burner(&d)),
        sw("burner_zec", |x: &mut A| x.burner_zec = thief_zec),
        sw("burner_coin", |x: &mut A| x.burner_coin = thief_coin),
        sw("amm_global_config", |x: &mut A| x.amm_global_config = pump::global()),
        sw("pool_authority", |x: &mut A| x.pool_authority = pump::pool_authority(&d)),
        sw("pool", |x: &mut A| x.pool = pump::pool(&d)),
        sw("pool_base_token_account", |x: &mut A| x.pool_base_token_account = thief_coin),
        sw("pool_quote_token_account", |x: &mut A| x.pool_quote_token_account = ata(&pump::pool(&d), &ZEC, &TOKEN)),
        sw("user_volume", |x: &mut A| x.user_volume = pump::user_volume(&kit::burner(&c))),
        sw("fee_config", |x: &mut A| x.fee_config = pump::fee_config()),
        sw("buyback_fee_recipient", |x: &mut A| x.buyback_fee_recipient = thief_zec),
        sw("amm_event_authority", |x: &mut A| x.amm_event_authority = fake),
        sw("amm_program", |x: &mut A| x.amm_program = nothing),
        sw("token_program", |x: &mut A| x.token_program = nothing),
        sw("token_2022_program", |x: &mut A| x.token_2022_program = nothing),
        sw("associated_token_program", |x: &mut A| x.associated_token_program = nothing),
        sw("system_program", |x: &mut A| x.system_program = nothing),
    ];
    let pot = w.ledger(&c).pots[3];
    let r = each_refused(&mut w, &payer, &[], good, burr::instruction::BurnPool {}, swaps);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(w.ledger(&c).pots[3] < pot, "with the right accounts the burn buys");
}

#[test]
fn stake() {
    let Stage { mut w, a, b, user, thief, nothing, .. } = stage();
    let good = w.stake_accounts(&user.pubkey(), &a);
    let thief_coin = ata(&thief.pubkey(), &a, &TOKEN_2022);
    let thief_key = thief.pubkey();
    type A = burr::accounts::StakeCoins;
    let swaps: Vec<Swap<A>> = vec![
        sw("mint", |x: &mut A| x.mint = b),
        sw("coin", |x: &mut A| x.coin = kit::coin(&b)),
        sw("ledger", |x: &mut A| x.ledger = kit::ledger(&b)),
        sw("authority", |x: &mut A| x.authority = kit::authority(&b)),
        sw("stake", |x: &mut A| x.stake = kit::stake(&a, &thief_key)),
        sw("stake_vault", |x: &mut A| x.stake_vault = thief_coin),
        sw("owner_coin", |x: &mut A| x.owner_coin = thief_coin),
        sw("token_2022_program", |x: &mut A| x.token_2022_program = nothing),
        sw("associated_token_program", |x: &mut A| x.associated_token_program = nothing),
        sw("system_program", |x: &mut A| x.system_program = nothing),
    ];
    let r = each_refused(&mut w, &user, &[], good, burr::instruction::Stake { amount: 10 }, swaps);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.stake_of(&a, &user.pubkey()).unwrap().amount, 1_010);
}

#[test]
fn unstake() {
    let Stage { mut w, a, b, user, thief, nothing, .. } = stage();
    // The thief has a stake of their own, to offer in place of the user's.
    assert!(w.stake(&thief, &a, 5).is_ok());
    w.b.wait(3_600);
    let good = w.unstake_accounts(&user.pubkey(), &a);
    let thief_zec = ata(&thief.pubkey(), &ZEC, &TOKEN);
    let thief_coin = ata(&thief.pubkey(), &a, &TOKEN_2022);
    let thief_key = thief.pubkey();
    type A = burr::accounts::UnstakeCoins;
    let swaps: Vec<Swap<A>> = vec![
        sw("mint", |x: &mut A| x.mint = b),
        sw("coin", |x: &mut A| x.coin = kit::coin(&b)),
        sw("ledger", |x: &mut A| x.ledger = kit::ledger(&b)),
        sw("authority", |x: &mut A| x.authority = kit::authority(&b)),
        sw("stake", |x: &mut A| x.stake = kit::stake(&a, &thief_key)),
        sw("stake (the same person's in another coin)", |x: &mut A| x.stake = kit::stake(&b, &x.owner)),
        sw("stake_vault", |x: &mut A| x.stake_vault = kit::stake_vault(&b)),
        sw("owner_coin", |x: &mut A| x.owner_coin = thief_coin),
        sw("vault", |x: &mut A| x.vault = kit::vault(&b)),
        sw("zec_mint", |x: &mut A| x.zec_mint = a),
        sw("owner_zec", |x: &mut A| x.owner_zec = thief_zec),
        sw("token_program", |x: &mut A| x.token_program = nothing),
        sw("token_2022_program", |x: &mut A| x.token_2022_program = nothing),
        sw("associated_token_program", |x: &mut A| x.associated_token_program = nothing),
        sw("system_program", |x: &mut A| x.system_program = nothing),
    ];
    let thief_zec_before = w.b.token_balance(&thief_zec);
    let thief_coin_before = w.b.token_balance(&thief_coin);
    let r = each_refused(&mut w, &user, &[], good, burr::instruction::Unstake { amount: 1_000 }, swaps);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.b.token_balance(&thief_zec), thief_zec_before);
    assert_eq!(w.b.token_balance(&thief_coin), thief_coin_before);
}

#[test]
fn king_buy_curve() {
    let Stage { mut w, a, b, user, thief, nothing, fake, .. } = stage();
    assert!(w.king_settle(&a).is_ok());
    let good = w.king_buy_curve_accounts(&user.pubkey(), &a);
    let thief_zec = ata(&thief.pubkey(), &ZEC, &TOKEN);
    let thief_coin = ata(&thief.pubkey(), &a, &TOKEN_2022);
    type A = burr::accounts::KingBuyCurve;
    let swaps: Vec<Swap<A>> = vec![
        sw("mint", |x: &mut A| x.mint = b),
        sw("coin", |x: &mut A| x.coin = kit::coin(&b)),
        sw("ledger", |x: &mut A| x.ledger = kit::ledger(&b)),
        sw("zec_mint", |x: &mut A| x.zec_mint = b),
        sw("buyer_coin", |x: &mut A| x.buyer_coin = thief_coin),
        sw("buyer_zec", |x: &mut A| x.buyer_zec = thief_zec),
        sw("pump_global", |x: &mut A| x.pump_global = pump::amm_global_config()),
        sw("bonding_curve", |x: &mut A| x.bonding_curve = pump::curve(&b)),
        sw("associated_base_bonding_curve", |x: &mut A| x.associated_base_bonding_curve = thief_coin),
        sw("associated_quote_bonding_curve", |x: &mut A| x.associated_quote_bonding_curve = thief_zec),
        sw("user_volume", |x: &mut A| x.user_volume = fake),
        sw("fee_config", |x: &mut A| x.fee_config = pda(&[b"fee_config", PUMP_AMM.as_ref()], &PUMP_FEES)),
        sw("buyback_fee_recipient", |x: &mut A| x.buyback_fee_recipient = thief_zec),
        sw("pump_event_authority", |x: &mut A| x.pump_event_authority = fake),
        sw("pump_program", |x: &mut A| x.pump_program = nothing),
        sw("token_program", |x: &mut A| x.token_program = nothing),
        sw("token_2022_program", |x: &mut A| x.token_2022_program = nothing),
        sw("associated_token_program", |x: &mut A| x.associated_token_program = nothing),
        sw("system_program", |x: &mut A| x.system_program = nothing),
    ];
    let data = burr::instruction::KingBuyCurve { quote_in: ZEC1 / 10, min_out: 1 };
    let r = each_refused(&mut w, &user, &[], good, data, swaps);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.ledger(&a).king.leader, user.pubkey());
    assert_eq!(w.ledger(&b).king.leader_amount, ZEC1 / 10, "coin B's round was not touched");
}

#[test]
fn king_buy_pool() {
    let Stage { mut w, c, d, user, thief, nothing, fake, .. } = stage();
    assert!(w.king_settle(&c).is_ok());
    let good = w.king_buy_pool_accounts(&user.pubkey(), &c);
    let thief_zec = ata(&thief.pubkey(), &ZEC, &TOKEN);
    let thief_coin = ata(&thief.pubkey(), &c, &TOKEN_2022);
    type A = burr::accounts::KingBuyPool;
    let swaps: Vec<Swap<A>> = vec![
        sw("mint", |x: &mut A| x.mint = d),
        sw("coin", |x: &mut A| x.coin = kit::coin(&d)),
        sw("ledger", |x: &mut A| x.ledger = kit::ledger(&d)),
        sw("zec_mint", |x: &mut A| x.zec_mint = d),
        sw("buyer_coin", |x: &mut A| x.buyer_coin = thief_coin),
        sw("buyer_zec", |x: &mut A| x.buyer_zec = thief_zec),
        sw("amm_global_config", |x: &mut A| x.amm_global_config = pump::global()),
        sw("pool_authority", |x: &mut A| x.pool_authority = pump::pool_authority(&d)),
        sw("pool", |x: &mut A| x.pool = pump::pool(&d)),
        sw("pool_base_token_account", |x: &mut A| x.pool_base_token_account = thief_coin),
        sw("pool_quote_token_account", |x: &mut A| x.pool_quote_token_account = thief_zec),
        sw("user_volume", |x: &mut A| x.user_volume = fake),
        sw("fee_config", |x: &mut A| x.fee_config = pump::fee_config()),
        sw("buyback_fee_recipient", |x: &mut A| x.buyback_fee_recipient = thief_zec),
        sw("amm_event_authority", |x: &mut A| x.amm_event_authority = fake),
        sw("amm_program", |x: &mut A| x.amm_program = nothing),
        sw("token_program", |x: &mut A| x.token_program = nothing),
        sw("token_2022_program", |x: &mut A| x.token_2022_program = nothing),
        sw("associated_token_program", |x: &mut A| x.associated_token_program = nothing),
        sw("system_program", |x: &mut A| x.system_program = nothing),
    ];
    let data = burr::instruction::KingBuyPool { quote_in: ZEC1 / 10, min_out: 1 };
    let r = each_refused(&mut w, &user, &[], good, data, swaps);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.ledger(&c).king.leader, user.pubkey());
}

#[test]
fn king_settle() {
    let Stage { mut w, a, b, user, thief, nothing, .. } = stage();
    let payer = w.b.payer.insecure_clone();
    let good = w.king_settle_accounts(&payer.pubkey(), &a, &user.pubkey());
    let thief_zec = ata(&thief.pubkey(), &ZEC, &TOKEN);
    let thief_key = thief.pubkey();
    type A = burr::accounts::KingSettle;
    let swaps: Vec<Swap<A>> = vec![
        sw("coin", |x: &mut A| x.coin = kit::coin(&b)),
        sw("ledger", |x: &mut A| x.ledger = kit::ledger(&b)),
        sw("authority", |x: &mut A| x.authority = kit::authority(&b)),
        sw("vault", |x: &mut A| x.vault = kit::vault(&b)),
        sw("zec_mint", |x: &mut A| x.zec_mint = a),
        sw("winner", |x: &mut A| {
            x.winner = thief_key;
            x.winner_zec = thief_zec;
        }),
        sw("winner_zec", |x: &mut A| x.winner_zec = thief_zec),
        sw("token_program", |x: &mut A| x.token_program = nothing),
        sw("associated_token_program", |x: &mut A| x.associated_token_program = nothing),
        sw("system_program", |x: &mut A| x.system_program = nothing),
    ];
    let pot = w.ledger(&a).pots[5];
    assert!(pot > 0);
    let before = w.zec_of(&user.pubkey());
    let thief_before = w.b.token_balance(&thief_zec);
    let r = each_refused(&mut w, &payer, &[], good, burr::instruction::KingSettle {}, swaps);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.zec_of(&user.pubkey()), before + pot);
    assert_eq!(w.b.token_balance(&thief_zec), thief_before);
}
