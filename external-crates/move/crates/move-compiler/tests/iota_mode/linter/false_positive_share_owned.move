// object has store but is never created externally
module a::has_store {
    use iota::transfer;
    use iota::object::UID;

    struct Obj has key, store {
        id: UID
    }

    public fun arg_object(o: Obj) {
        let arg = o;
        transfer::public_share_object(arg);
    }
}

// object is created locally, but the analysis cannot determine that currently
module a::cannot_determine_to_be_new {
    use iota::transfer;
    use iota::object::UID;

    struct Obj has key {
        id: UID
    }

    struct Obj2 has key {
        id: UID
    }

    // we do not do interprodedural analysis here
    fun make_obj(o: Obj2, ctx: &mut iota::tx_context::TxContext): Obj {
        transfer::transfer(o, @0);
        Obj { id: iota::object::new(ctx) }
    }

    public fun transfer(o2: Obj2, ctx: &mut iota::tx_context::TxContext) {
        let o = make_obj(o2, ctx);
        transfer::transfer(o, iota::tx_context::sender(ctx));
    }

    public fun share(o2: Obj2, ctx: &mut iota::tx_context::TxContext) {
        let o = make_obj(o2, ctx); // cannot determine this is local because of `X`
        transfer::share_object(o);
    }
}

module iota::tx_context {
    struct TxContext has drop {}
    public fun sender(_: &TxContext): address {
        @0
    }
}

module iota::object {
    const ZERO: u64 = 0;
    struct UID has store {
        id: address,
    }
    public fun delete(_: UID) {
        abort ZERO
    }
    public fun new(_: &mut iota::tx_context::TxContext): UID {
        abort ZERO
    }
}

module iota::transfer {
    const ZERO: u64 = 0;
    public fun transfer<T: key>(_: T, _: address) {
        abort ZERO
    }
    public fun share_object<T: key>(_: T) {
        abort ZERO
    }
    public fun public_share_object<T: key>(_: T) {
        abort ZERO
    }
}
