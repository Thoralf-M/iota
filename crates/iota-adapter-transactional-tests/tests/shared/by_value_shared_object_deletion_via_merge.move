// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

//# init --addresses t1=0x0 t2=0x0 --accounts A

// Merge:
// shared into owned -> Can do anything, SO is deleted
// owned into shared -> SO restrictions apply (DF, DOF, Transfer, Freeze, Delete)
// shared into shared -> SO restrictions apply (DF, DOF, Transfer, Freeze, Delete)

// tfer, then abort

//# publish

module t2::o2 {
    use iota::dynamic_field as df;
    use iota::dynamic_object_field as dof;
    use iota::iota::IOTA;
    use iota::coin::{Self, Coin};

    public struct Obj2 has key, store {
        id: UID,
    }

    public fun mint_shared_coin(ctx: &mut TxContext) {
        transfer::public_share_object(coin::zero<IOTA>(ctx))
    }

    public fun mint_shared_obj(ctx: &mut TxContext) {
        transfer::public_share_object(Obj2 { id: object::new(ctx) });
    }

    public fun mint_owned_coin(ctx: &mut TxContext) {
        transfer::public_transfer(coin::zero<IOTA>(ctx), @A)
    }

    public fun deleter(o2: Coin<IOTA>) {
        coin::destroy_zero(o2);
    }

    public fun freezer(o2: Coin<IOTA>) {
        transfer::public_freeze_object(o2);
    }

    public fun dofer(parent: &mut Obj2, o2: Coin<IOTA>) {
        dof::add(&mut parent.id, 0, o2);
    }

    public fun dfer(parent: &mut Obj2, o2: Coin<IOTA>) {
        df::add(&mut parent.id, 0, o2);
    }

    public fun transferrer(o2: Coin<IOTA>) {
        transfer::public_transfer(o2, @0x0);
    }

    public fun sharer(o2: Coin<IOTA>) {
        transfer::public_share_object(o2);
    }
}

//# run t2::o2::mint_shared_coin

//# run t2::o2::mint_owned_coin

//# view-object 2,0

//# view-object 3,0

// Merge shared into owned -- after that the shared object is deleted and its
// just a normal owned object
//# programmable --sender A --inputs object(2,0) object(3,0)
//> 0: MergeCoins(Input(1), [Input(0)]);
//> 1: t2::o2::transferrer(Input(1));

// **Merge owned into shared**

//# run t2::o2::mint_owned_coin

//# run t2::o2::mint_shared_coin

//# run t2::o2::mint_shared_obj

//# view-object 7,0

//# view-object 8,0

//# view-object 9,0

// Merge and then delete it -- should work fine
//# programmable --sender A --inputs object(7,0) object(8,0) object(9,0)
//> 0: MergeCoins(Input(1), [Input(0)]);
//> 1: t2::o2::deleter(Input(1));

// **Merge shared into shared**

//# run t2::o2::mint_shared_coin

//# run t2::o2::mint_shared_coin

//# run t2::o2::mint_shared_obj

//# view-object 14,0

//# view-object 15,0

//# view-object 16,0

// Merge and then delete it -- should work fine
//# programmable --sender A --inputs object(14,0) object(15,0) object(16,0)
//> 0: MergeCoins(Input(1), [Input(0)]);
//> 1: t2::o2::deleter(Input(1));
