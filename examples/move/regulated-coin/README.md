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

### Send Tokens across chains using PTB

```shell
export OFT_OBJECT_ID=0x1def6c8815e500c6b7e846179cb48baf33b053b56e5e66543a30272b1236f9e1 # get from previous txs
export OFT_OAPP_ID=0x5601995b1c80f87c61c2932e15e0b60ec0362b8243ad12cc5bcb110c8b2215f0 # get from previous txs
export MESSAGING_CHANNEL_ID=0xe015e1346f7a20115cd2cff453406d0db2fe64fc209dc74b3df31a9fad3c4069 # get from previous txs
export DST_EID=40423 # iotal1-testnet endpoint id
export TO_ADDRESS=$(iota client active-address) # destination address on the dst_eid chain
export REFUND_ADDRESS=$(iota client active-address) # refund address on source chain
export REGULATED_COIN_TO_SEND=$( \
  iota client objects --json | \
  jq -r --arg pkg "$REGULATED_COIN_PACKAGE_ID" \
  '[.[] | select(.data.type == "0x2::coin::Coin<\($pkg)::regulated_coin::REGULATED_COIN>") | .data.objectId] | first' \
)
export AMOUNT_TO_SPLIT=10  # amount to split from the coin (must be >= AMOUNT_LD)
export AMOUNT_LD=10  # amount to send in local decimals
export MIN_AMOUNT_LD=9  # minimum amount to receive (slippage protection)
export NATIVE_FEE=1000000000  # native fee amount (1 IOTA)
export UTILS_PACKAGE_ID=0x379b562468eed5cf259a2f279527f92d231e52bb260c5169230b0a87f6a52c82 # layerzero testnet
export ZRO_COIN_PACKAGE_ID=0x50e04dda960d432cc5f98f5c51ec65e73d313f0e6c1ad13146215946524a56f3 # layerzero testnet
export ENDPOINT_V2_PACKAGE_ID=0xfca1ac6ffcae8ce9d937e94f30c930f9ce295b29496ed975d272efec511e2495 # layerzero testnet
export ENDPOINT_V2_OBJECT_ID=0x63c99ce9839a3259f2299666157f639882e4911250ee3016d190fa6944561f98 # layerzero testnet
export ULN302_PACKAGE_ID=0xf87812112d8ad8329269d7445be936057651dcf96a692f32ee1d8de82296cc7d # layerzero testnet
export ULN302_OBJECT_ID=0xca3eb88711d4ab5587605439ea5b968d2ba1908b9162f34e9f116e5ec7edeb16 # layerzero testnet
export EXECUTOR_WORKER_PACKAGE_ID=0xab64bfcce4c5c357019e3b215f2502b6c7e60b6879ff5718240c7c157885708f # layerzero testnet
export EXECUTOR_WORKER_OBJECT_ID=0xa2eb5f1df687dc901dbcc1bf174a105fac158d1fac633cd109e800c0923d8a55 # layerzero testnet
export EXECUTOR_FEE_LIB_PACKAGE_ID=0xda132e9bedd921ce3b2f3efeab87f3a348671c2b9994347166dc522a078b8507 # layerzero testnet
export EXECUTOR_FEE_LIB_OBJECT_ID=0x4164680bd255efae23fd420687e162ec50523f0bd55d0d87de33bd96c59a6396 # layerzero testnet
export PRICE_FEED_PACKAGE_ID=0xe23152186cc998c2a1c8f2ed07d6607a69efd8b0ab4594789dd99fd49761f7f7 # layerzero testnet
export PRICE_FEED_OBJECT_ID=0xfbb2014cd2babdc54d33c5980d13de169a55722f9ad686180438a6cd7b536928 # layerzero testnet
export DVN_PACKAGE_ID=0xcf9c9179230ef3dfde78f8ad3749060b530c8b5ca53fde8af9c1f016276583f6 # layerzero testnet
export DVN_OBJECT_ID=0x38eae904a49f930f7115ff1b6470715e5ba0f55cdf8d4b49a3e3793eff9d2427 # layerzero testnet
export DVN_FEE_LIB_PACKAGE_ID=0x3c585acf9b51d0ad7cca084a11bb9a27b0d283075f0bbaccc04abc442201108d # layerzero testnet
export DVN_FEE_LIB_OBJECT_ID=0xea02e95cef21b9e511ab4ceb79831ec6be44a67300e2d52c559b6a346ac37d5c # layerzero testnet
export PRICE_FEED_SHARED_OBJECT_ID=0xfbb2014cd2babdc54d33c5980d13de169a55722f9ad686180438a6cd7b536928 # layerzero testnet
export LAYERZERO_TREASURY=0x172c0be00589891ab0e788400d07a283e921f4c5be2eb576b9c9028667f429db # layerzero testnet
# Extra options for LayerZero execution (gas limit for lzReceive)
# Format: [0,3,1,0,17,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1] represents gas=1, value=0
# pnpm add @layerzerolabs/lz-v2-utilities
# node -e "
# import { Options } from '@layerzerolabs/lz-v2-utilities';
# console.log(Options.newOptions().addExecutorLzReceiveOption(1, 0).toBytes())
# "

iota client ptb \
--split-coins @$REGULATED_COIN_TO_SEND "[10]" \
--assign split_coin \
--assign to_address_bytes vector"[161,169,125,32,187,173,121,226,172,137,242,21,163,179,196,242,255,154,26,163,204,38,229,41,189,230,231,188,85,0,214,16]" \
--move-call $UTILS_PACKAGE_ID::bytes32::from_bytes to_address_bytes \
--assign to_bytes32 \
--assign extra_options_vec vector"[0, 3, 1, 0, 17, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]" \
--move-call $OFT_PACKAGE_ID::send_param::create $DST_EID to_bytes32 $AMOUNT_LD $MIN_AMOUNT_LD extra_options_vec '""' '""' \
--assign send_param \
--move-call $OFT_PACKAGE_ID::oft_sender::tx_sender \
--assign tx_sender \
--split-coins gas "[$NATIVE_FEE]" \
--assign native_fee_coin \
--move-call std::option::none "<0x2::coin::Coin<$ZRO_COIN_PACKAGE_ID::zro::ZRO>>" \
--assign none_zro_coin \
--assign refund_addresses  "some(@$REFUND_ADDRESS)" \
--move-call $OFT_PACKAGE_ID::oft::send @$OFT_OBJECT_ID @$OFT_OAPP_ID tx_sender send_param split_coin.0 native_fee_coin.0 none_zro_coin refund_addresses @$REGULATED_COIN_TREASURY @$DENY_LIST_OBJECT_ID @0x6 \
--assign send_result \
--move-call $ENDPOINT_V2_PACKAGE_ID::endpoint_v2::send @$ENDPOINT_V2_OBJECT_ID @$MESSAGING_CHANNEL_ID send_result.0 \
--assign endpoint_send_result \
--move-call $ULN302_PACKAGE_ID::uln_302::send @$ULN302_OBJECT_ID endpoint_send_result.0 \
--assign uln_send_result \
--move-call $EXECUTOR_WORKER_PACKAGE_ID::executor_worker::assign_job @$EXECUTOR_WORKER_OBJECT_ID uln_send_result.0 \
--assign executor_assign_result \
--move-call $EXECUTOR_FEE_LIB_PACKAGE_ID::executor_fee_lib::get_fee @$EXECUTOR_FEE_LIB_OBJECT_ID executor_assign_result.0 \
--assign executor_get_fee_result \
--move-call $PRICE_FEED_PACKAGE_ID::price_feed::estimate_fee_by_eid @$PRICE_FEED_OBJECT_ID executor_get_fee_result.0 \
--assign executor_estimate_fee_result \
--move-call $EXECUTOR_FEE_LIB_PACKAGE_ID::executor_fee_lib::confirm_get_fee @$EXECUTOR_FEE_LIB_OBJECT_ID executor_assign_result.0 executor_get_fee_result.0 \
--assign executor_confirm_fee_result \
--move-call $EXECUTOR_WORKER_PACKAGE_ID::executor_worker::confirm_assign_job @$EXECUTOR_WORKER_OBJECT_ID uln_send_result.0 executor_assign_result.0 \
--assign executor_confirm_assign_result \
--move-call $DVN_PACKAGE_ID::dvn::assign_job @$DVN_OBJECT_ID uln_send_result.1 \
--assign dvn_assign_result \
--move-call $DVN_FEE_LIB_PACKAGE_ID::dvn_fee_lib::get_fee @$DVN_FEE_LIB_OBJECT_ID dvn_assign_result.0 \
--assign dvn_get_fee_result \
--move-call $PRICE_FEED_PACKAGE_ID::price_feed::estimate_fee_by_eid @$PRICE_FEED_SHARED_OBJECT_ID dvn_get_fee_result.0 \
--move-call $DVN_FEE_LIB_PACKAGE_ID::dvn_fee_lib::confirm_get_fee @$DVN_FEE_LIB_OBJECT_ID dvn_assign_result.0 dvn_get_fee_result.0 \
--assign dvn_confirm_fee_result \
--move-call $DVN_PACKAGE_ID::dvn::confirm_assign_job @$DVN_OBJECT_ID uln_send_result.1 dvn_assign_result.0 \
--assign dvn_confirm_assign_result \
--move-call $ULN302_PACKAGE_ID::uln_302::confirm_send @$ULN302_OBJECT_ID @$ENDPOINT_V2_OBJECT_ID @$LAYERZERO_TREASURY @$MESSAGING_CHANNEL_ID send_result.0 endpoint_send_result.0 uln_send_result.0 uln_send_result.1 \
--assign uln_confirm_result \
--move-call $OFT_PACKAGE_ID::oft::confirm_send @$OFT_OBJECT_ID @$OFT_OAPP_ID tx_sender send_result.0 send_result.1 \
--assign confirm_result \
--move-call std::option::destroy_none "<0x2::coin::Coin<0x2::iota::IOTA>>" confirm_result.2 \
--move-call std::option::destroy_none "<0x2::coin::Coin<$ZRO_COIN_PACKAGE_ID::zro::ZRO>>" confirm_result.3 \
--transfer-objects "[split_coin.0]" @$(iota client active-address) \
--dry-run # remove --dry-run for actual execution
```

Example transaction: https://explorer.iota.org/txblock/6486zvFXSjPFtZ18uEAei9bw7JH4uVCtjRiJGnHyHsfA?network=testnet
LayerZero scan: https://testnet.layerzeroscan.com/tx/6486zvFXSjPFtZ18uEAei9bw7JH4uVCtjRiJGnHyHsfA

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
--assign packet_header vector"[1,0,0,0,0,0,0,0,1,0,0,157,231,60,91,85,132,203,133,45,102,141,31,185,61,94,35,147,160,84,3,121,74,39,66,76,87,119,23,244,75,231,228,19,68,0,0,157,231,60,91,85,132,203,133,45,102,141,31,185,61,94,35,147,160,84,3,121,74,39,66,76,87,119,23,244,75,231,228,19,68]" \
--move-call $UTILS_PACKAGE_ID::bytes32::from_address @$PAYLOAD_HASH_HEX \
--assign payload_hash \
--move-call $ULN302_PACKAGE_ID::uln_302::commit_verification @$ULN302_OBJECT_ID @$VERIFICATION_OBJECT_ID @$ENDPOINT_V2_OBJECT_ID @$MESSAGING_CHANNEL_ID packet_header payload_hash @0x6 \
--dry-run # remove --dry-run for actual execution
```
tx: https://explorer.iota.org/txblock/2m6iSgHmUy37xphLvxrueMjgoN6CC6K9r4mahCBxBYEH?network=testnet


