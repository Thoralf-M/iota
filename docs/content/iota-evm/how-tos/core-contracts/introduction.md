---
description: The ISC Core Contracts allows VMs to access ISC functionality.
image: /img/logo/WASP_logo_dark.png
tags:
  - evm
  - magic
  - solidity
  - rpc
teams:
  - iotaledger/l2-smart-contract
---

# The Core Contracts

The [core contracts](../../explanations/core-contracts.md) are contracts deployed on every chain and are vital to interact with L1 and the chain itself. They can be called in Solidity through the [ISC Magic Contract](../../references/magic-contract/introduction.md).

## The ISC Magic Contract

The Magic contract is an EVM contract deployed by default on every ISC chain, in the EVM genesis block, at
address `0x1074000000000000000000000000000000000000`.
The implementation of the Magic contract is baked-in in
the [`evm`](../../references/core-contracts/evm.md) [core contract](../../references/core-contracts/overview.md);
i.e. it is not a pure-Solidity contract.

The Magic contract has several methods, which are categorized into specialized
interfaces: `ISCSandbox`, `ISCAccounts`, `ISCUtil` and so on.
You can access these interfaces from any Solidity contract by importing
the [ISC library](https://www.npmjs.com/package/@iota/iscmagic):

```bash npm2yarn
npm install @iota/iscmagic
```

You can import it into your contracts like this:

```solidity
import "@iota/iscmagic/ISC.sol";
```

The Magic contract also provides proxy ERC20 contracts to manipulate ISC base
tokens and coins on L2.

:::info ERC20Coin

ERC20 coin proxy contracts can currently only be registered by ChainAdmin. We are looking into changing that in a future update.

:::

:::info Reference Docs

If you need further info about magic contracts interfaces you can check out the [magic contract docs](../../references/magic-contract/introduction.md).

:::

## Call a Function

:::info Ease of use

To make it easier for developers to use the core contracts, you should, in most cases, run the functions from the magic contract directly. For example, to get the native token balance, you could [call the `balanceBaseToken()`](call-view.md) directly with `callView`, or use [`getL2BalanceBaseTokens`](basics/get-balance.md) of the magic contract, or (the suggested way) register your native token as [`ERC20`] and call the standard [`balanceof`] function. What you use also depends on what you optimize for. For example, to save gas, it could be interesting for you to call core contracts from your favorite web3 library directly and compute other things off-chain.

:::

In the example below, `ISC.sandbox.getEntropy()` calls the
[`getEntropy`](https://github.com/iotaledger/wasp/blob/develop/packages/vm/core/evm/iscmagic/ISCSandbox.sol#L20)
method of the `ISCSandbox` interface, which, in turn,
calls [ISC Sandbox's](../../explanations/sandbox.md) `GetEntropy`.

```solidity
pragma solidity >=0.8.5;

import "@iota/iscmagic/ISC.sol";

contract MyEVMContract {
    event EntropyEvent(bytes32 entropy);

    // this will emit a "random" value taken from the ISC entropy value
    function emitEntropy() public {
        bytes32 e = ISC.sandbox.getEntropy();
        emit EntropyEvent(e);
    }
}
```
