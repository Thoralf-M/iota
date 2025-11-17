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
# Initialize OFT, example tx CsjGVLUXTQcfKssbGVudKiZBrCjD1kpsyRhBAFqowQgm
SHARED_DECIMALS=6  # Choose appropriate shared decimals (≤ local decimals)
iota client ptb \
--move-call $OFT_PACKAGE_ID::oft_impl::init_oft @$OFT_INIT_TICKET_ID @$OFT_OAPP_ID @$SUPPLY_MANAGER_CAP_ID @$REGULATED_COIN_TREASURY $SHARED_DECIMALS \
--assign admin_cap_migration_cap \
--transfer-objects "[admin_cap_migration_cap.0, admin_cap_migration_cap.1]" @$(iota client active-address) \
--dry-run # remove --dry-run for actual execution
```
tx: https://explorer.iota.org/txblock/CsjGVLUXTQcfKssbGVudKiZBrCjD1kpsyRhBAFqowQgm?network=testnet
OFT object: https://explorer.iota.org/object/0x1def6c8815e500c6b7e846179cb48baf33b053b56e5e66543a30272b1236f9e1?network=testnet

After initialization, register the OApp with the LayerZero endpoint to enable cross-chain messaging.

```shell
export OFT_OBJECT_ID=0x1def6c8815e500c6b7e846179cb48baf33b053b56e5e66543a30272b1236f9e1 # replace with created object ID
export OAPP_ADMIN_CAP=0x44244683bafcc1b53eb71d6cb7ad08d0619686446ecb4b601ab8e7efa98e49bf # replace with created object ID
export ENDPOINT_V2=0x63c99ce9839a3259f2299666157f639882e4911250ee3016d190fa6944561f98 # layerzero testnet endpoint
export OFT_COMPOSE_MANAGER_OBJECT_ID=0x0f0b3c80ed9bfc559a4018fc37e3fefc690814fdfbb5125d7219768c7ca5a1f6 # layerzero testnet compose manager
iota client ptb \
--move-call $OFT_PACKAGE_ID::oft_ptb_builder::lz_receive_info @$OFT_OBJECT_ID @$ENDPOINT_V2 @$OFT_COMPOSE_MANAGER_OBJECT_ID @0x6 @$REGULATED_COIN_TREASURY @0x403 \
--assign lz_receive_info \
--move-call $OFT_PACKAGE_ID::oft::register_oapp @$OFT_OBJECT_ID @$OFT_OAPP_ID @$OAPP_ADMIN_CAP @$ENDPOINT_V2 lz_receive_info \
--dry-run # remove --dry-run for actual execution
```
tx: https://explorer.iota.org/txblock/31FYzeLxKvy8C7YfBBbXqKUgAZotgp65bMQA9Yfts4KF?network=testnet

After registering the OApp, set the peer for the destination chain to enable messaging.

```shell
export MESSAGING_CHANNEL_ID=0xe015e1346f7a20115cd2cff453406d0db2fe64fc209dc74b3df31a9fad3c4069 # get from previous tx
export OAPP_PACKAGE_ID=0x05fb5547cce6f480ea92d9b77c9ca7056080c89896ddb394f26eb6db3fa9fdb6 # OApp package ID from LayerZero testnet deployments
export UTILS_PACKAGE_ID=0x379b562468eed5cf259a2f279527f92d231e52bb260c5169230b0a87f6a52c82 # layerzero testnet utils package
export DST_EID=40423 # iotal1-testnet endpoint id, just setting destination to same chain for testing
iota client ptb \
--move-call $UTILS_PACKAGE_ID::bytes32::from_address @$OFT_PACKAGE_ID \
--assign peer_bytes32 \
--move-call $OAPP_PACKAGE_ID::oapp::set_peer @$OFT_OAPP_ID @$OAPP_ADMIN_CAP @$ENDPOINT_V2 @$MESSAGING_CHANNEL_ID $DST_EID peer_bytes32 \
--dry-run
```
tx: https://explorer.iota.org/txblock/8AHGNBbxkyLFnQL9PN9sN2hJtAxT4m61RAZ3iNG1zxj?network=testnet

After setting the peer, the OFT is ready for cross-chain transfers. Note: Full LayerZero setup (endpoints, DVNs, executors) is required for actual cross-chain functionality, which is beyond the scope of this guide. Refer to the LayerZero documentation for complete setup.

iotal1-testnet is 40423 https://docs.layerzero.network/v2/deployments/deployed-contracts

<!-- TODO: update PTB to same as SDK example or have an SDK example only
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
``` -->

### Quote and Send Tokens across chains
Send tx with modified SDK (node scripts/send.js):

Provide bytes to sign command:
```shell
iota keytool sign --address 0xa1a97d20bbad79e2ac89f215a3b3c4f2ff9a1aa3cc26e529bde6e7bc5500d610 --data 
```

https://explorer.iota.org/txBlock/2dmcc3dqxsVvM6MuJc6fM4qJhAGvogUttDcbXeEAzeR6?network=https%3A%2F%2Findexer.testnet.iota.cafe

https://testnet.layerzeroscan.com/tx/2dmcc3dqxsVvM6MuJc6fM4qJhAGvogUttDcbXeEAzeR6



### Manual commit verification
Commit verification for ULN302 (usually not required to be done manually):

Get packet header with data fetched from the transaction PacketSentEvent https://explorer.iota.org/txblock/C75tiQrUahK34q7u8tNJhK3sHj3awudL436B1rYtHK2d?network=testnet:
First 81 bytes are the packet header
PAYLOAD_HASH_HEX extracted from the dynamic field https://explorer.iota.org/object/0xf4a743ae7a44e3e24c8f00c6164da8a790e7723db572d9e780bbcd3e15096bd6?network=testnet that was created by the verify function called by the DVN https://explorer.iota.org/txblock/AFQXgZw6H1ww1nS2a5WrYu3ybgGbrGjzab9EXH2fQYEu?network=testnet

```shell
ULN302_PACKAGE_ID=0xf87812112d8ad8329269d7445be936057651dcf96a692f32ee1d8de82296cc7d
VERIFICATION_OBJECT_ID=0x898a41148ba0b90e7de598d95775ec886aa961ae3ee7a35436760a1736dce085
ULN302_OBJECT_ID=0xca3eb88711d4ab5587605439ea5b968d2ba1908b9162f34e9f116e5ec7edeb16
ENDPOINT_V2_OBJECT_ID=0x63c99ce9839a3259f2299666157f639882e4911250ee3016d190fa6944561f98
MESSAGING_CHANNEL_ID=0x8d4baf8842469c38a74f410df8622d3078bc4b2cdc8148b75ddc27015fb4af63
UTILS_PACKAGE_ID=0x379b562468eed5cf259a2f279527f92d231e52bb260c5169230b0a87f6a52c82
PAYLOAD_HASH_HEX=0x835202f55ffba65a466707193c590f4d142e14a2959cece701344a3e2c7e177a
iota client ptb \
--make-move-vec "<u8>" "[1,0,0,0,0,0,0,0,1,0,0,157,231,60,91,85,132,203,133,45,102,141,31,185,61,94,35,147,160,84,3,121,74,39,66,76,87,119,23,244,75,231,228,19,68,0,0,157,231,60,91,85,132,203,133,45,102,141,31,185,61,94,35,147,160,84,3,121,74,39,66,76,87,119,23,244,75,231,228,19,68]" \
--assign packet_header \
--move-call $UTILS_PACKAGE_ID::bytes32::from_address @$PAYLOAD_HASH_HEX \
--assign payload_hash \
--move-call $ULN302_PACKAGE_ID::uln_302::commit_verification @$ULN302_OBJECT_ID @$VERIFICATION_OBJECT_ID @$ENDPOINT_V2_OBJECT_ID @$MESSAGING_CHANNEL_ID packet_header payload_hash @0x6 \
--dry-run # remove --dry-run for actual execution
```
tx: https://explorer.iota.org/txblock/2m6iSgHmUy37xphLvxrueMjgoN6CC6K9r4mahCBxBYEH?network=testnet


