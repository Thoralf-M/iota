// Modules with infinite loops to stress the ID leak verifier
module a::m {
    use iota::object::{Self, UID};
    use iota::tx_context::TxContext;

    struct Obj has key {
        id: UID,
    }

    public entry fun loop_forever(ctx: &mut TxContext) {
        let obj = Obj { id:  object::new(ctx) };
        let Obj { id: uid } = obj;
        loop {
            object::delete(uid);
            uid = object::new(ctx);
        }
    }

    public entry fun loop_forever_2(ctx: &mut TxContext) {
        let obj = Obj { id:  object::new(ctx) };
        let Obj { id: uid } = obj;
        loop {
            object::delete(uid);
            obj = Obj { id: object::new(ctx) };
            loop {
                Obj { id: uid } = obj;
                object::delete(uid);
                uid = object::new(ctx);
                break
            }
        }
    }
}


module iota::object {
    struct UID has store {
        id: address,
    }
    public fun new(_: &mut iota::tx_context::TxContext): UID {
        abort 0
    }
    public fun delete(_: UID) {
        abort 0
    }
}

module iota::tx_context {
    struct TxContext has drop {}
    public fun sender(_: &TxContext): address {
        @0
    }
}

module iota::transfer {
    public fun transfer<T: key>(_: T, _: address) {
        abort 0
    }
}
