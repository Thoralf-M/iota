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

OFT object: https://explorer.iota.org/object/0xd44a36aa6f5eea5d610cad6e014c9deba5db1a18bf9b08935aecc501baea1706?network=testnet

After initialization, register the OApp with the LayerZero endpoint to enable cross-chain messaging.

```shell
export ENDPOINT_V2=0x63c99ce9839a3259f2299666157f639882e4911250ee3016d190fa6944561f98
export OAPP_ADMIN_CAP=0x55b47f9db475638b33ac550757ca14d4e59244bbc15c1266c7d7f3e5eb6f113b
# TODO provide actual lz_receive_info
iota client ptb \
--move-call $OFT_PACKAGE_ID::oft::register_oapp @$OFT_OBJECT_ID @$OFT_OAPP_ID @$OAPP_ADMIN_CAP @$ENDPOINT_V2 '""' \
--dry-run # remove --dry-run for actual execution
```
tx: https://explorer.iota.org/txblock/9dqLKrJasNHVJCB5NYzHDm9GqBbdpq1bSpAWSwPYnYVa?network=testnet

After registering the OApp, set the peer for the destination chain to enable messaging.

```shell
export MESSAGING_CHANNEL_ID=0x7446c2f6685267e14b9215cb0acdc9391c7b5585466e6ed756e54fc1f19b54c0
export OAPP_PACKAGE_ID=0x05fb5547cce6f480ea92d9b77c9ca7056080c89896ddb394f26eb6db3fa9fdb6 # OApp package ID from LayerZero testnet deployments
export DST_EID=40423 # iotal1-testnet endpoint id, just setting destination to same chain for testing
iota client ptb \
--move-call $UTILS_PACKAGE_ID::bytes32::from_address @$TO_ADDRESS \
--assign peer_bytes32 \
--move-call $OAPP_PACKAGE_ID::oapp::set_peer @$OFT_OAPP_ID @$OAPP_ADMIN_CAP @$ENDPOINT_V2 @$MESSAGING_CHANNEL_ID $DST_EID peer_bytes32 \
--dry-run
```
tx: https://explorer.iota.org/txblock/FUtMGCNFoFwiXJPAs7BRhZWTv2E2Tg9V2mnxrmu67jXx?network=testnet

After setting the peer, the OFT is ready for cross-chain transfers. Note: Full LayerZero setup (endpoints, DVNs, executors) is required for actual cross-chain functionality, which is beyond the scope of this guide. Refer to the LayerZero documentation for complete setup.

iotal1-testnet is 40423 https://docs.layerzero.network/v2/deployments/deployed-contracts

```shell
export OFT_OBJECT_ID=0xd44a36aa6f5eea5d610cad6e014c9deba5db1a18bf9b08935aecc501baea1706
export OFT_OAPP_ID=0x558a6e246bd80f89965a7e68c92984849db1568ba4e9e1b2e70973c098b30584
export DST_EID=40423 # iotal1-testnet endpoint id
export TO_ADDRESS=$(iota client active-address) # destination address on the dst_eid chain
export REGULATED_COIN_TO_SEND=0x175da63567acade04d1b9b2f81e521e912239d10c2b7c13ac571399c9dc2d917
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
# just errors with const EUnauthorized: u64 = 10;
# --assign sml_call \
# --move-call $SIMPLE_MESSAGE_LIB_PACKAGE_ID::simple_message_lib::send @$SML_OBJECT_ID @$ENDPOINT_V2_OBJECT_ID @$MESSAGING_CHANNEL_ID send_result.0 sml_call \
# --dry-run
--move-call $OFT_PACKAGE_ID::oft::confirm_send @$OFT_OBJECT_ID @$OFT_OAPP_ID tx_sender send_result.0 send_result.1 \
--dry-run # remove --dry-run for actual execution
```


```shell
export DVN=0x38eae904a49f930f7115ff1b6470715e5ba0f55cdf8d4b49a3e3793eff9d2427
```
0x43713bc8ac3c792aeba65d0894c20ff57f65bb81bd6522a0b167518e4fa0b2a3::package_whitelist_validator::Validator https://explorer.iota.org/object/0x1270e2859c59f141e618d4e39bd75aaba244e6da01ed4df1cfaa8b4522dd767d?network=testnet
