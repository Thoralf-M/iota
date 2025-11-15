# Regulated Coin

The packages folder contains three packages:
- `regulated_coin`: This package contains the implementation of a regulated coin, which includes features like minting, burning, pausing transfers and managing a deny list.
- `supply_manager`: This package contains an example of how the `SupplyManagerCap` from the `regulated_coin` can be used by another package to manage the supply of it.
- `oft`: This package provides an implementation of the Omnichain Fungible Token (OFT) Standard for the regulated coin. The Omnichain Fungible Token (OFT) Standard enables fungible tokens to exist across multiple blockchains while maintaining a unified supply. The OFT standard works by debiting an amount of tokens from a sender on the source chain and crediting the same amount of tokens to a receiver on the destination chain.

The following sections show the steps to manage everything with a single address.
For multisig operations, look at [multisig management](./multisig-management.mdx).

## Prepare the environment

Install the IOTA CLI https://docs.iota.org/developer/getting-started/install-iota and set up the environment:
```shell
iota client switch --env testnet
iota client faucet
```

## Publish the packages

### 1. Publish `regulated_coin` package

```shell
# Save JSON output for later parsing (choose any path you prefer)
REGULATED_PUBLISH_JSON=publish-outputs/publish_regulated_coin.json
iota client publish packages/regulated_coin --json | tee $REGULATED_PUBLISH_JSON
```

### 2. (Optional) Publish `supply_manager` package

```shell
SUPPLY_MANAGER_PUBLISH_JSON=publish-outputs/publish_supply_manager.json
iota client publish packages/supply_manager --json | tee $SUPPLY_MANAGER_PUBLISH_JSON
```

### 3. (Optional) Publish `oft` package

```shell
OFT_PUBLISH_JSON=publish-outputs/publish_oft.json
iota client publish packages/oft --json | tee $OFT_PUBLISH_JSON
```

### 4. Extract IDs from the saved JSON

Extract/export all environment variables via a helper script.

```shell
chmod +x ./scripts/extract-env.sh
# After publishing transactions (see steps below) run:
./scripts/extract-env.sh publish-outputs/publish_regulated_coin.json publish-outputs/publish_supply_manager.json publish-outputs/publish_oft.json

# To load into current shell
source <(./scripts/extract-env.sh publish-outputs/publish_regulated_coin.json publish-outputs/publish_supply_manager.json publish-outputs/publish_oft.json)
```

Variables extracted:
- `REGULATED_COIN_PACKAGE_ID`: package id
- `REGULATED_COIN_ADMIN_CAP`: controls the Treasury
- `REGULATED_COIN_TREASURY`: shared Treasury object
Optional for supply manager package:
 - `SUPPLY_MANAGER_PACKAGE_ID`: package id of the `supply_manager`
 - `SUPPLY_MANAGER_ADMIN_CAP`: admin capability object for managing the SupplyManager wrapper
 - `SUPPLY_MANAGER_OBJECT_ID`: shared SupplyManager wrapper object
Optional for oft package:
 - `OFT_PACKAGE_ID`: package id of the `oft`
 - `OFT_INIT_TICKET_ID`: OFTInitTicket object for initializing the OFT
 - `OFT_OAPP_ID`: shared OApp object for the OFT


## Call functions

You should now have the required variables (`REGULATED_COIN_ADMIN_CAP`, `REGULATED_COIN_TREASURY`, `REGULATED_COIN_PACKAGE_ID`, and optionally supply manager vars). Additionally set these convenience variables (replace addresses if needed):
```shell
SUPPLY_MANAGER_ADDRESS=$(iota client active-address)
RECIPIENT_ADDRESS=$(iota client active-address)
DENY_LIST_OBJECT_ID=0x403
```

### Regulated Coin Operations

Create a SupplyManager:
```shell
# 1) Create a SupplyManagerCap and transfer it to the SupplyManager address
iota client ptb \
--move-call $REGULATED_COIN_PACKAGE_ID::regulated_coin::new_supply_manager @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP \
--assign sm_cap \
--transfer-objects "[sm_cap]" @$SUPPLY_MANAGER_ADDRESS
sleep 2
SUPPLY_MANAGER_CAP_ID=$(iota client objects --json | jq -r '[.[] | select(.data.type != null and (.data.type | test("SupplyManagerCap"))) | .data.objectId] | first')
echo "SupplyManagerCap ID: $SUPPLY_MANAGER_CAP_ID"
```

Block and unblock an address:
```shell
BLOCKED_ADDRESS=0xC0FFEE
# Block
iota client ptb \
--move-call $REGULATED_COIN_PACKAGE_ID::regulated_coin::block_address @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP @$DENY_LIST_OBJECT_ID @$BLOCKED_ADDRESS
# Unblock
iota client ptb \
--move-call $REGULATED_COIN_PACKAGE_ID::regulated_coin::unblock_address @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP @$DENY_LIST_OBJECT_ID @$BLOCKED_ADDRESS
```

Global pause and unpause transfers:
```shell
# Pause (effective from next epoch for receiving)
iota client ptb \
--move-call $REGULATED_COIN_PACKAGE_ID::regulated_coin::pause_transfers @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP @$DENY_LIST_OBJECT_ID
# Unpause
iota client ptb \
--move-call $REGULATED_COIN_PACKAGE_ID::regulated_coin::unpause_transfers @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP @$DENY_LIST_OBJECT_ID
```

Mint and burn via regulated_coin (direct, using SupplyManagerCap):
```shell
# Mint 100 to recipient; sender must be the holder of the SupplyManagerCap
AMOUNT=100
iota client ptb \
--move-call $REGULATED_COIN_PACKAGE_ID::regulated_coin::mint @$REGULATED_COIN_TREASURY @$SUPPLY_MANAGER_CAP_ID @$DENY_LIST_OBJECT_ID $AMOUNT @$RECIPIENT_ADDRESS

# Burn a coin; sender must own the coin being burned (and present the SupplyManagerCap)
COIN_TO_BURN=$(iota client objects --json | jq -r '[.[] | select(.data.type != null and (.data.type | test("Coin<.*regulated_coin::REGULATED_COIN"))) | .data.objectId] | first')
echo "Burning: $COIN_TO_BURN"
iota client ptb \
--move-call $REGULATED_COIN_PACKAGE_ID::regulated_coin::burn @$REGULATED_COIN_TREASURY @$SUPPLY_MANAGER_CAP_ID @$DENY_LIST_OBJECT_ID @$COIN_TO_BURN
```

Revoke SupplyManager (need to re-authorize or create a new SupplyManagerCap afterwards to be able to mint/burn again):
```shell
iota client ptb \
--move-call $REGULATED_COIN_PACKAGE_ID::regulated_coin::unauthorize_supply_manager @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP
```

Re-authorize SupplyManager
```shell
iota client ptb \
--move-call $REGULATED_COIN_PACKAGE_ID::regulated_coin::authorize_supply_manager @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP @$SUPPLY_MANAGER_CAP_ID
```

### SupplyManager Operations

```shell
# Attach the SupplyManagerCap to the shared SupplyManager wrapper (admin only)
iota client ptb \
--move-call $SUPPLY_MANAGER_PACKAGE_ID::supply_manager::add_supply_manager_cap @$SUPPLY_MANAGER_OBJECT_ID @$SUPPLY_MANAGER_ADMIN_CAP @$SUPPLY_MANAGER_CAP_ID

# Mint via wrapper (admin only)
iota client ptb \
--move-call $SUPPLY_MANAGER_PACKAGE_ID::supply_manager::mint @$SUPPLY_MANAGER_OBJECT_ID @$SUPPLY_MANAGER_ADMIN_CAP @$REGULATED_COIN_TREASURY @$DENY_LIST_OBJECT_ID 100 @$RECIPIENT_ADDRESS

# Burn via wrapper (admin only; the PTB must include the coin object being burned)
COIN_TO_BURN=$(iota client objects --json | jq -r '[.[] | select(.data.type != null and (.data.type | test("Coin<.*regulated_coin::REGULATED_COIN"))) | .data.objectId] | first')
echo "Burning: $COIN_TO_BURN"
iota client ptb \
--move-call $SUPPLY_MANAGER_PACKAGE_ID::supply_manager::burn @$SUPPLY_MANAGER_OBJECT_ID @$SUPPLY_MANAGER_ADMIN_CAP @$REGULATED_COIN_TREASURY @$DENY_LIST_OBJECT_ID @$COIN_TO_BURN

# Remove the attached SupplyManagerCap from the wrapper and transfer it out (admin only)
iota client ptb \
--move-call $SUPPLY_MANAGER_PACKAGE_ID::supply_manager::remove_supply_manager_cap @$SUPPLY_MANAGER_OBJECT_ID @$SUPPLY_MANAGER_ADMIN_CAP \
--assign sm_cap \
--transfer-objects "[sm_cap]" @$SUPPLY_MANAGER_ADDRESS
```

### Sending a regulated coin

Send a specific amount of a coin object:
```shell
AMOUNT_TO_SEND=10
COIN_INPUT=$(iota client objects --json | jq -r '[.[] | select(.data.type != null and (.data.type | test("Coin<.*regulated_coin::REGULATED_COIN"))) | .data.objectId] | first')
echo "Sending $AMOUNT_TO_SEND to $RECIPIENT_ADDRESS with coin $COIN_INPUT"
iota client pay \
--input-coins $COIN_INPUT \
--recipients $RECIPIENT_ADDRESS \
--amounts $AMOUNT_TO_SEND 
```

Send a full coin object:
```shell
COIN_INPUT=$(iota client objects --json | jq -r '[.[] | select(.data.type != null and (.data.type | test("Coin<.*regulated_coin::REGULATED_COIN"))) | .data.objectId] | first')
echo "Sending $COIN_INPUT to $RECIPIENT_ADDRESS"
iota client transfer --object-id $COIN_INPUT --to $RECIPIENT_ADDRESS
```

### OFT Operations

After publishing the `oft` package, initialize the OFT using the OFTInitTicket and the shared OApp. You need to have created a SupplyManagerCap beforehand.

```shell
# Initialize OFT, example tx 5J9i4ZNjgw1cmTDRSWQDfVtpUPZDtqU9vmGsHbRdrrRg
SHARED_DECIMALS=6  # Choose appropriate shared decimals (≤ local decimals)
iota client ptb \
--move-call $OFT_PACKAGE_ID::oft_impl::init_oft @$OFT_INIT_TICKET_ID @$OFT_OAPP_ID @$SUPPLY_MANAGER_CAP_ID @$REGULATED_COIN_TREASURY $SHARED_DECIMALS \
--assign admin_cap_migration_cap \
--transfer-objects "[admin_cap_migration_cap.0, admin_cap_migration_cap.1]" @$(iota client active-address) \
--dry-run # remove --dry-run for actual execution
```
tx: https://explorer.iota.org/txblock/9yX1dPiNUQ3ZgLWpJNscrec6iDyVFQo2dnMFaCK17yc3?network=testnet
OFT object: https://explorer.iota.org/object/0xf060b3831835ce348aedb7edb5245a7ab2b7bf0f0af187a3e2a996994225fedd?network=testnet

After initialization, register the OApp with the LayerZero endpoint to enable cross-chain messaging.

```shell
export ENDPOINT_V2=0x63c99ce9839a3259f2299666157f639882e4911250ee3016d190fa6944561f98
export OAPP_ADMIN_CAP=0x0d0f7a60fa4c69a18cdfb3d34e8d5386c228624e0dd3c4a3d8d389e34b103e7a
export OFT_OBJECT_ID=0xf060b3831835ce348aedb7edb5245a7ab2b7bf0f0af187a3e2a996994225fedd
iota client ptb \
--move-call $OFT_PACKAGE_ID::oft::register_oapp @$OFT_OBJECT_ID @$OFT_OAPP_ID @$OAPP_ADMIN_CAP @$ENDPOINT_V2 '""' \
--dry-run # remove --dry-run for actual execution
```
tx: https://explorer.iota.org/txblock/HDsSK9xSwoKHYvWwsHa8ryqcK5EToQfhMY8jmkgH6ntb?network=testnet

After registering the OApp, set the peer for the destination chain to enable messaging.

```shell
export MESSAGING_CHANNEL_ID=0x8d4baf8842469c38a74f410df8622d3078bc4b2cdc8148b75ddc27015fb4af63
export OAPP_PACKAGE_ID=0x05fb5547cce6f480ea92d9b77c9ca7056080c89896ddb394f26eb6db3fa9fdb6 # OApp package ID from LayerZero testnet deployments
export DST_EID=40423 # iotal1-testnet endpoint id, just setting destination to same chain for testing
iota client ptb \
--move-call $UTILS_PACKAGE_ID::bytes32::from_address @$OFT_PACKAGE_ID \
--assign peer_bytes32 \
--move-call $OAPP_PACKAGE_ID::oapp::set_peer @$OFT_OAPP_ID @$OAPP_ADMIN_CAP @$ENDPOINT_V2 @$MESSAGING_CHANNEL_ID $DST_EID peer_bytes32 \
--dry-run
```
tx: https://explorer.iota.org/txblock/A8NQrkguAz5Xzg8ZxpPW4Gh2CnV3c54S17UUgHBSciAx?network=testnet

After setting the peer, the OFT is ready for cross-chain transfers. Note: Full LayerZero setup (endpoints, DVNs, executors) is required for actual cross-chain functionality, which is beyond the scope of this guide. Refer to the LayerZero documentation for complete setup.

iotal1-testnet is 40423 https://docs.layerzero.network/v2/deployments/deployed-contracts

TODO: update PTB to same as SDK example or have an SDK example only
```shell
export OFT_OBJECT_ID=0xf060b3831835ce348aedb7edb5245a7ab2b7bf0f0af187a3e2a996994225fedd
export DST_EID=40423 # iotal1-testnet endpoint id
export TO_ADDRESS=$(iota client active-address) # destination address on the dst_eid chain
export REGULATED_COIN_TO_SEND=$( \
  iota client objects --json | \
  jq -r --arg pkg "$REGULATED_COIN_PACKAGE_ID" \
  '[.[] | select(.data.type == "0x2::coin::Coin<\($pkg)::regulated_coin::REGULATED_COIN>") | .data.objectId] | first' \
)  # get an IOTA coin for fees
export AMOUNT_LD=10  # amount to send in local decimals
export MIN_AMOUNT_LD=10  # minimum amount to receive
export NATIVE_FEE_COIN_ID=$(iota client objects --json | jq -r '[.[] | select(.data.type == "0x2::coin::Coin<0x2::iota::IOTA>") | .data.objectId] | first')  # get an IOTA coin for fees
export ENDPOINT_PACKAGE_ID=0xfca1ac6ffcae8ce9d937e94f30c930f9ce295b29496ed975d272efec511e2495 # LayerZero endpoint package ID on testnet
export UTILS_PACKAGE_ID=0x379b562468eed5cf259a2f279527f92d231e52bb260c5169230b0a87f6a52c82 # LayerZero utils package ID on testnet
export ZRO_COIN_PACKAGE_ID=0x50e04dda960d432cc5f98f5c51ec65e73d313f0e6c1ad13146215946524a56f3
export ENDPOINT_V2_OBJECT_ID=0x63c99ce9839a3259f2299666157f639882e4911250ee3016d190fa6944561f98
export ENDPOINT_V2_PACKAGE_ID=0xfca1ac6ffcae8ce9d937e94f30c930f9ce295b29496ed975d272efec511e2495
export SIMPLE_MESSAGE_LIB_PACKAGE_ID=0x1ac164f11ef54614d01b8f41fa5bfbf61654216cd8b323a00991cd63879afbb5
export SML_OBJECT_ID=0xa48db6ccef9ebce87df4871f9a0490e1e0004425b04ef82e380de16d77bbd68c
iota client ptb \
--move-call $UTILS_PACKAGE_ID::bytes32::from_address @$TO_ADDRESS \
--assign to_bytes32 \
--move-call $OFT_PACKAGE_ID::send_param::create $DST_EID to_bytes32 $AMOUNT_LD $MIN_AMOUNT_LD '""' '""' '""' \
--assign send_param \
--move-call $OFT_PACKAGE_ID::oft_sender::tx_sender \
--assign tx_sender \
--move-call std::option::none "<0x2::coin::Coin<$ZRO_COIN_PACKAGE_ID::zro::ZRO>>" \
--assign none_zro_coin \
--move-call $OFT_PACKAGE_ID::oft::send @$OFT_OBJECT_ID @$OFT_OAPP_ID tx_sender send_param @$REGULATED_COIN_TO_SEND @$NATIVE_FEE_COIN_ID none_zro_coin none @$REGULATED_COIN_TREASURY @$DENY_LIST_OBJECT_ID @0x6 \
--assign send_result \
--move-call $ENDPOINT_V2_PACKAGE_ID::endpoint_v2::send @$ENDPOINT_V2_OBJECT_ID @$MESSAGING_CHANNEL_ID send_result.0 \
--assign sml_call \
--move-call $SIMPLE_MESSAGE_LIB_PACKAGE_ID::simple_message_lib::send @$SML_OBJECT_ID @$ENDPOINT_V2_OBJECT_ID @$MESSAGING_CHANNEL_ID send_result.0 sml_call \
--move-call $OFT_PACKAGE_ID::oft::confirm_send @$OFT_OBJECT_ID @$OFT_OAPP_ID tx_sender send_result.0 send_result.1 \
--assign confirm_result \
--transfer-objects "[confirm_result.2, confirm_result.3]" @$(iota client active-address) \
--dry-run # remove --dry-run for actual execution
```

Send tx with modified SDK:
https://explorer.iota.org/txblock/C75tiQrUahK34q7u8tNJhK3sHj3awudL436B1rYtHK2d?network=https%3A%2F%2Findexer.testnet.iota.cafe
https://testnet.layerzeroscan.com/tx/C75tiQrUahK34q7u8tNJhK3sHj3awudL436B1rYtHK2d

```JS
import { OFT } from "@layerzerolabs/lz-iotal1-oft-sdk-v2";
import { SDK, validateTransaction } from "@layerzerolabs/lz-iotal1-sdk-v2";
import { Transaction } from "@iota/iota-sdk/transactions";
import { IotaClient } from '@iota/iota-sdk/client';
import { Stage } from "@layerzerolabs/lz-definitions"
import { toBase64 } from '@iota/bcs';
import { Options } from '@layerzerolabs/lz-v2-utilities';

const iotaClient = new IotaClient({
    url: 'https://indexer.testnet.iota.cafe',
});

// Initialize LayerZero protocol SDK
const protocolSDK = new SDK({
    client: iotaClient,
    stage: Stage.TESTNET,
});

const oftPackageId = '0x3c5b5584cb852d668d1fb93d5e2393a05403794a27424c577717f44be7e41344'

// Create OFT instance (with optional parameters for convenience)
const oft = new OFT(protocolSDK, oftPackageId);

const senderAddress = '0xa1a97d20bbad79e2ac89f215a3b3c4f2ff9a1aa3cc26e529bde6e7bc5500d610'
// Prepare send parameters
const sendParam = {
    dstEid: 40423, // Destination endpoint ID
    to: (() => { const arr = new Uint8Array(32); arr.set(Buffer.from(senderAddress.slice(2), 'hex')); return arr; })(), // Recipient address as Uint8Array (32 bytes)
    amountLd: 10n, // Amount in local decimals
    minAmountLd: 9n, // Minimum amount (slippage protection)
    extraOptions: Options.newOptions().addExecutorLzReceiveOption(1, 0).toBytes(),// new Uint8Array(0), // LayerZero execution options
    composeMsg: new Uint8Array(0), // Optional compose message
    oftCmd: new Uint8Array(0), // Optional OFT command (unused in default OFT)
};

// Quote the transfer fees
// const messagingFee = await oft.quoteSend(
//     senderAddress,
//     sendParam,
//     false, // payInZro: false = pay in native token
// );

// Execute the transfer
const tx = new Transaction();

// Split coins from sender's wallet
// const coin = await oft.splitCoinMoveCall(tx, senderAddress, sendParam.amountLd);
const coin = await oft.splitCoinMoveCall(tx, senderAddress, BigInt(20));

try {

    // Send the tokens
    await oft.sendMoveCall(
        tx,
        senderAddress,
        sendParam,
        coin,
        // messagingFee.nativeFee,
        // messagingFee.zroFee,
        1000000000,
        0,
        senderAddress, // refund address
    );


    // Transfer any remaining coins back to sender
    tx.transferObjects([coin], senderAddress);
    tx.setSender(senderAddress);
    tx.setGasBudget(1000000000);
    // let bytes = await tx.build({ client: iotaClient });
    // console.log(bytes)
    let transactionBytes = toBase64(await tx.build({ client: iotaClient }));
    console.log(transactionBytes)
    let dryRun = await iotaClient.dryRunTransactionBlock({ transactionBlock: transactionBytes })
    console.log(dryRun.effects.status)
} catch (e) {
    console.error(e)
}
```

Provide bytes to sign command:
```shell
iota keytool sign --address 0xa1a97d20bbad79e2ac89f215a3b3c4f2ff9a1aa3cc26e529bde6e7bc5500d610 --data 
```

