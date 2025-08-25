module regulated_coin::regulated_coin;

use iota::coin;
use regulated_coin::treasury;

public struct REGULATED_COIN has drop {}

fun init(witness: REGULATED_COIN, ctx: &mut TxContext) {
    let decimals = 9;
    let symbol = b"REGULATED_COIN";
    let name = b"Regulated Coin";
    let description = b"Example Regulated Coin";
    let icon_url = option::none();
    let allow_global_pause = true;

    let (treasury_cap, deny_cap, metadata) = coin::create_regulated_currency_v1(
        witness,
        decimals,
        symbol,
        name,
        description,
        icon_url,
        allow_global_pause,
        ctx,
    );

    treasury::new(
        treasury_cap, 
        deny_cap,
        metadata,
        ctx
    );
}

#[test_only]
public fun test_init(ctx: &mut TxContext) {
    let witness = REGULATED_COIN {};
    init(witness, ctx);
}
