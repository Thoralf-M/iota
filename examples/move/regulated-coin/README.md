# Regulated Coin

The packages folder contains two packages:
- `regulated_coin`: This package contains the implementation of a regulated coin, which includes features like minting, burning, pausing transfers and managing a deny list.
- `supply_manager`: This package contains an example of how the `SupplyManagerCap` from the `regulated_coin` can be used by another package to manage the supply of it.

The following sections show the steps to manage everything with a single address.
For multisig operations, look at [multisig management](./multisig-management.mdx).

## Prepare the environment

Install the IOTA CLI https://docs.iota.org/developer/getting-started/install-iota and set up the environment:
```shell
iota client switch --env devnet
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

### 3. Extract IDs from the saved JSON

Extract/export all environment variables via a helper script.

```shell
chmod +x ./scripts/extract-env.sh
# After publishing transactions (see steps below) run:
./scripts/extract-env.sh publish-outputs/publish_regulated_coin.json publish-outputs/publish_supply_manager.json

# To load into current shell
source <(./scripts/extract-env.sh publish-outputs/publish_regulated_coin.json publish-outputs/publish_supply_manager.json)
```

Variables extracted:
- `REGULATED_COIN_PACKAGE_ID`: package id
- `REGULATED_COIN_ADMIN_CAP`: controls the Treasury
- `REGULATED_COIN_TREASURY`: shared Treasury object
Optional for supply manager package:
 - `SUPPLY_MANAGER_PACKAGE_ID`: package id of the `supply_manager`
 - `SUPPLY_MANAGER_ADMIN_CAP`: admin capability object for managing the SupplyManager wrapper
 - `SUPPLY_MANAGER_OBJECT_ID`: shared SupplyManager wrapper object


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
--move-call $REGULATED_COIN_PACKAGE_ID::treasury::new_supply_manager "<$REGULATED_COIN_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP \
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
--move-call $REGULATED_COIN_PACKAGE_ID::treasury::block_address "<$REGULATED_COIN_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP @$DENY_LIST_OBJECT_ID @$BLOCKED_ADDRESS
# Unblock
iota client ptb \
--move-call $REGULATED_COIN_PACKAGE_ID::treasury::unblock_address "<$REGULATED_COIN_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP @$DENY_LIST_OBJECT_ID @$BLOCKED_ADDRESS
```

Global pause and unpause transfers:
```shell
# Pause (effective from next epoch for receiving)
iota client ptb \
--move-call $REGULATED_COIN_PACKAGE_ID::treasury::pause_transfers "<$REGULATED_COIN_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP @$DENY_LIST_OBJECT_ID
# Unpause
iota client ptb \
--move-call $REGULATED_COIN_PACKAGE_ID::treasury::unpause_transfers "<$REGULATED_COIN_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP @$DENY_LIST_OBJECT_ID
```

Mint and burn via regulated_coin (direct, using SupplyManagerCap):
```shell
# Mint 100 to recipient; sender must be the holder of the SupplyManagerCap
AMOUNT=100
iota client ptb \
--move-call $REGULATED_COIN_PACKAGE_ID::treasury::mint "<$REGULATED_COIN_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$REGULATED_COIN_TREASURY @$SUPPLY_MANAGER_CAP_ID @$DENY_LIST_OBJECT_ID $AMOUNT @$RECIPIENT_ADDRESS

# Burn a coin; sender must own the coin being burned (and present the SupplyManagerCap)
COIN_TO_BURN=$(iota client objects --json | jq -r '[.[] | select(.data.type != null and (.data.type | test("Coin<.*regulated_coin::REGULATED_COIN"))) | .data.objectId] | first')
echo "Burning: $COIN_TO_BURN"
iota client ptb \
--move-call $REGULATED_COIN_PACKAGE_ID::treasury::burn "<$REGULATED_COIN_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$REGULATED_COIN_TREASURY @$SUPPLY_MANAGER_CAP_ID @$DENY_LIST_OBJECT_ID @$COIN_TO_BURN
```

Revoke SupplyManager (need to re-authorize or create a new SupplyManagerCap afterwards to be able to mint/burn again):
```shell
iota client ptb \
--move-call $REGULATED_COIN_PACKAGE_ID::treasury::unauthorize_supply_manager "<$REGULATED_COIN_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP
```

Re-authorize SupplyManager
```shell
iota client ptb \
--move-call $REGULATED_COIN_PACKAGE_ID::treasury::authorize_supply_manager "<$REGULATED_COIN_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP @$SUPPLY_MANAGER_CAP_ID
```

### SupplyManager Operations

```shell
# Attach the SupplyManagerCap to the shared SupplyManager wrapper (admin only)
iota client ptb \
--move-call $SUPPLY_MANAGER_PACKAGE_ID::supply_manager::add_supply_manager_cap "<$REGULATED_COIN_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$SUPPLY_MANAGER_OBJECT_ID @$SUPPLY_MANAGER_ADMIN_CAP @$SUPPLY_MANAGER_CAP_ID

# Mint via wrapper (admin only)
iota client ptb \
--move-call $SUPPLY_MANAGER_PACKAGE_ID::supply_manager::mint "<$REGULATED_COIN_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$SUPPLY_MANAGER_OBJECT_ID @$SUPPLY_MANAGER_ADMIN_CAP @$REGULATED_COIN_TREASURY @$DENY_LIST_OBJECT_ID 100 @$RECIPIENT_ADDRESS

# Burn via wrapper (admin only; the PTB must include the coin object being burned)
COIN_TO_BURN=$(iota client objects --json | jq -r '[.[] | select(.data.type != null and (.data.type | test("Coin<.*regulated_coin::REGULATED_COIN"))) | .data.objectId] | first')
echo "Burning: $COIN_TO_BURN"
iota client ptb \
--move-call $SUPPLY_MANAGER_PACKAGE_ID::supply_manager::burn "<$REGULATED_COIN_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$SUPPLY_MANAGER_OBJECT_ID @$SUPPLY_MANAGER_ADMIN_CAP @$REGULATED_COIN_TREASURY @$DENY_LIST_OBJECT_ID @$COIN_TO_BURN

# Remove the attached SupplyManagerCap from the wrapper and transfer it out (admin only)
iota client ptb \
--move-call $SUPPLY_MANAGER_PACKAGE_ID::supply_manager::remove_supply_manager_cap "<$REGULATED_COIN_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$SUPPLY_MANAGER_OBJECT_ID @$SUPPLY_MANAGER_ADMIN_CAP \
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
