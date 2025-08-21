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

```shell
iota client publish regulated_coin --display object_changes
```

The command output looks like in [publish_regulated_coin](./command-outputs/publish_regulated_coin.txt).

Running
```shell
iota client objects
```
should now show the `AdminCap` and `UpgradeCap` objects for the `regulated_coin` package (together with some 0x0000..0002::coin::Coin objects):
```
│ ╭────────────┬──────────────────────────────────────────────────────────────────────╮ │
│ │ objectId   │  0x5c4bee40c68dcb8239b4dfdbec3f9aaffda823ce614d277e859eae77343fbb7d  │ │
│ │ version    │  3                                                                   │ │
│ │ digest     │  09yANnt36R9lhL7FBvWyrOFcjQvvPKFuUmy5J7lutu4=                        │ │
│ │ objectType │  0xd5c6..8a09::treasury::AdminCap                                    │ │
│ ╰────────────┴──────────────────────────────────────────────────────────────────────╯ │
│ ╭────────────┬──────────────────────────────────────────────────────────────────────╮ │
│ │ objectId   │  0xaf4475b7331e6c6ae96a53dcc155ee9cc713cb7244ec8e32c88e5d3ef5b1ba15  │ │
│ │ version    │  3                                                                   │ │
│ │ digest     │  AQszm1iVNVNMJnIOoaJw0NzYYiB70PKI1wBE4/b/D4w=                        │ │
│ │ objectType │  0x0000..0002::package::UpgradeCap                                   │ │
│ ╰────────────┴──────────────────────────────────────────────────────────────────────╯ │
```

The `AdminCap` is used to control the `Treasury` and the `UpgradeCap` is required to upgrade the package in the future.

Next publish the `supply_manager` package:

```shell
iota client publish supply_manager --display object_changes
```

The command output looks like in [publish_supply_manager](./command-outputs/publish_supply_manager.txt).

```shell
iota client objects
```
should now additionally show the `SupplyManagerAdminCap` and an additional `UpgradeCap` object for the `supply_manager` package:
```
│ ╭────────────┬──────────────────────────────────────────────────────────────────────╮ │
│ │ objectId   │  0x51e0a426e7126109a57272e8b4c754560c6206a38b04b0ab3ce3aaf1272d67e9  │ │
│ │ version    │  4                                                                   │ │
│ │ digest     │  oXAcVcOeN+bUTdOLoVhPD/UpgKuujTB4Tx6X0Q7o/Gs=                        │ │
│ │ objectType │  0x0000..0002::package::UpgradeCap                                   │ │
│ ╰────────────┴──────────────────────────────────────────────────────────────────────╯ │
│ ╭────────────┬──────────────────────────────────────────────────────────────────────╮ │
│ │ objectId   │  0x674b6297bc070b297656acfbffe84b92384e3061c975dc1b5d8e1afc96dd609d  │ │
│ │ version    │  4                                                                   │ │
│ │ digest     │  XX+UnMuoJ5lRJFb3vDKQzJ21B4gF0H/IaKpmQrJsd7U=                        │ │
│ │ objectType │  0xb1e2..172d::supply_manager::SupplyManagerAdminCap                 │ │
│ ╰────────────┴──────────────────────────────────────────────────────────────────────╯ │
```

## Call functions

To call functions of the `regulated_coin` package, you need to have the `AdminCap` and `Treasury` object ids.
For the `Treasury` object id, look find it in the publish output like [publish_regulated_coin](./command-outputs/publish_regulated_coin.txt) by searching for `::treasury::Treasury<`
```
│  ┌──                                                                                                                                                                                                       │
│  │ ObjectID: 0x6ea985ca00537399b837a97305c4835289046c4b72d5c615f9100f3dfd9bc924                                                                                                                            │
│  │ Sender: 0x111111111504e9350e635d65cd38ccd2c029434c6a3a480d8947a9ba6a15b215                                                                                                                              │
│  │ Owner: Shared( 3 )                                                                                                                                                                                      │
│  │ ObjectType: 0xd5c682c054ebf6bd05712f7c12977cd8bf73032200db9a5cfad30406c0458a09::treasury::Treasury<0xd5c682c054ebf6bd05712f7c12977cd8bf73032200db9a5cfad30406c0458a09::regulated_coin::REGULATED_COIN>  │
│  │ Version: 3                                                                                                                                                                                              │
│  │ Digest: BEGCFX17z6ZjDwF1igFRHKRM9pgVCciZazVeqdcMPdTW                                                                                                                                                    │
│  └──    
```
We also need the package id, but that's just the part before `::treasury::Treasury`, so in this example `0xd5c682c054ebf6bd05712f7c12977cd8bf73032200db9a5cfad30406c0458a09`.

### Regulated Coin Operations

First run the following commands to set the required variables:
```shell
REGULATED_COIN_ADMIN_CAP=0x5c4bee40c68dcb8239b4dfdbec3f9aaffda823ce614d277e859eae77343fbb7d
REGULATED_COIN_TREASURY=0x6ea985ca00537399b837a97305c4835289046c4b72d5c615f9100f3dfd9bc924
TREASURY_PACKAGE_ID=0xd5c682c054ebf6bd05712f7c12977cd8bf73032200db9a5cfad30406c0458a09
# Optional: your addresses
SUPPLY_MANAGER_ADDRESS=$(iota client active-address)
RECIPIENT_ADDRESS=$(iota client active-address)
DENY_LIST_OBJECT_ID=0x403
```

Create supply manager:
```shell
# 1) Create a SupplyManagerCap and transfer it to the supply manager address
iota client ptb \
--move-call $TREASURY_PACKAGE_ID::treasury::new_supply_manager "<$TREASURY_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP \
--assign sm_cap \
--transfer-objects "[sm_cap]" @$SUPPLY_MANAGER_ADDRESS
sleep 2
# Save the cap id if needed for later revocation
SUPPLY_MANAGER_CAP_ID=$(iota client objects --json | jq -r '[.[] | select(.data.type != null and (.data.type | test("SupplyManagerCap"))) | .data.objectId] | first')
echo "SupplyManagerCap ID: $SUPPLY_MANAGER_CAP_ID"
```

Block and unblock an address:
```shell
BLOCKED_ADDRESS=0xC0FFEE
# Block
iota client ptb \
--move-call $TREASURY_PACKAGE_ID::treasury::block_address "<$TREASURY_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP @$DENY_LIST_OBJECT_ID @$BLOCKED_ADDRESS
# Unblock
iota client ptb \
--move-call $TREASURY_PACKAGE_ID::treasury::unblock_address "<$TREASURY_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP @$DENY_LIST_OBJECT_ID @$BLOCKED_ADDRESS
```

Global pause and unpause transfers:
```shell
# Pause (effective from next epoch for receiving)
iota client ptb \
--move-call $TREASURY_PACKAGE_ID::treasury::pause_transfers "<$TREASURY_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP @$DENY_LIST_OBJECT_ID
# Unpause
iota client ptb \
--move-call $TREASURY_PACKAGE_ID::treasury::unpause_transfers "<$TREASURY_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP @$DENY_LIST_OBJECT_ID
```

Mint and burn via regulated_coin (direct, using SupplyManagerCap):
```shell
# Mint 100 to recipient; sender must be the holder of the SupplyManagerCap
AMOUNT=100
iota client ptb \
--move-call $TREASURY_PACKAGE_ID::treasury::mint "<$TREASURY_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$REGULATED_COIN_TREASURY @$SUPPLY_MANAGER_CAP_ID @$DENY_LIST_OBJECT_ID $AMOUNT @$RECIPIENT_ADDRESS

# Burn a coin; sender must own the coin being burned (and present the SupplyManagerCap)
COIN_TO_BURN=$(iota client objects --json | jq -r '[.[] | select(.data.type != null and (.data.type | test("Coin<.*regulated_coin::REGULATED_COIN"))) | .data.objectId] | first')
echo "Burning: $COIN_TO_BURN"
iota client ptb \
--move-call $TREASURY_PACKAGE_ID::treasury::burn "<$TREASURY_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$REGULATED_COIN_TREASURY @$SUPPLY_MANAGER_CAP_ID @$DENY_LIST_OBJECT_ID @$COIN_TO_BURN
```

Revoke supply manager (need to create a new SupplyManagerCap afterwards to be able to mint again):
```shell
iota client ptb \
--move-call $TREASURY_PACKAGE_ID::treasury::unauthorize_supply_manager "<$TREASURY_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$REGULATED_COIN_TREASURY @$REGULATED_COIN_ADMIN_CAP @$SUPPLY_MANAGER_CAP_ID
```

### SupplyManager Operations

First run the following commands to set the required variables:
```shell
SUPPLY_MANAGER_PACKAGE_ID=0xb1e25f9c4880743bcc90f56f7beab5e83de896aa6ba72bd7b7feb5fdaa37172d
SUPPLY_MANAGER_OBJECT_ID=0x9ff63b82a70d5e81b2ce06f4816c8ed82863e8a5ccebb54b627887f0eecc53c3
SUPPLY_MANAGER_ADMIN_CAP=0x674b6297bc070b297656acfbffe84b92384e3061c975dc1b5d8e1afc96dd609d
```

```shell
# Attach the SupplyManagerCap to the shared SupplyManager wrapper (admin only)
iota client ptb \
--move-call $SUPPLY_MANAGER_PACKAGE_ID::supply_manager::add_supply_manager_cap "<$TREASURY_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$SUPPLY_MANAGER_OBJECT_ID @$SUPPLY_MANAGER_ADMIN_CAP @$SUPPLY_MANAGER_CAP_ID

# Mint via wrapper (admin only)
iota client ptb \
--move-call $SUPPLY_MANAGER_PACKAGE_ID::supply_manager::mint "<$TREASURY_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$SUPPLY_MANAGER_OBJECT_ID @$SUPPLY_MANAGER_ADMIN_CAP @$REGULATED_COIN_TREASURY @$DENY_LIST_OBJECT_ID 100 @$RECIPIENT_ADDRESS

# Burn via wrapper (admin only; the PTB must include the coin object being burned)
COIN_TO_BURN=$(iota client objects --json | jq -r '[.[] | select(.data.type != null and (.data.type | test("Coin<.*regulated_coin::REGULATED_COIN"))) | .data.objectId] | first')
echo "Burning: $COIN_TO_BURN"
iota client ptb \
--move-call $SUPPLY_MANAGER_PACKAGE_ID::supply_manager::burn "<$TREASURY_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$SUPPLY_MANAGER_OBJECT_ID @$SUPPLY_MANAGER_ADMIN_CAP @$REGULATED_COIN_TREASURY @$DENY_LIST_OBJECT_ID @$COIN_TO_BURN

# Remove the attached SupplyManagerCap from the wrapper and transfer it out (admin only)
iota client ptb \
--move-call $SUPPLY_MANAGER_PACKAGE_ID::supply_manager::remove_supply_manager_cap "<$TREASURY_PACKAGE_ID::regulated_coin::REGULATED_COIN>" @$SUPPLY_MANAGER_OBJECT_ID @$SUPPLY_MANAGER_ADMIN_CAP \
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
--input-coins $COIN_TO_SEND \
--recipients $RECIPIENT_ADDRESS \
--amounts $AMOUNT_TO_SEND 
```

Send a full coin object:
```shell
COIN_INPUT=$(iota client objects --json | jq -r '[.[] | select(.data.type != null and (.data.type | test("Coin<.*regulated_coin::REGULATED_COIN"))) | .data.objectId] | first')
echo "Sending $COIN_INPUT to $RECIPIENT_ADDRESS"
iota client transfer --object-id $COIN_INPUT --to $RECIPIENT_ADDRESS
```
