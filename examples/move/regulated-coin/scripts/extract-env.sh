#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   ./scripts/extract-env.sh publish_regulated_coin.json [publish_supply_manager.json] [publish_oft.json]
# You can pass one, two, or three files (regulated first, optional supply manager second, optional oft third).
# The script prints export commands you can eval.
# Example:
#   ./scripts/extract-env.sh publish-outputs/publish_regulated_coin.json \
#       publish-outputs/publish_supply_manager.json publish-outputs/publish_oft.json | tee extracted.vars
#   source <(./scripts/extract-env.sh publish-outputs/publish_regulated_coin.json)

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required" >&2
  exit 1
fi

REG_FILE="${1:-}"
SUP_FILE="${2:-}"
OFT_FILE="${3:-}"

if [[ -z "$REG_FILE" || ! -f "$REG_FILE" ]]; then
  echo "First argument must be regulated_coin publish JSON file" >&2
  exit 1
fi

# Extract regulated coin values
REGULATED_COIN_ADMIN_CAP=$(jq -r '.objectChanges[] | select((.objectType // "") | test("::regulated_coin::AdminCap$")) | .objectId' "$REG_FILE" | head -n1)
REGULATED_COIN_TREASURY=$(jq -r '.objectChanges[] | select((.objectType // "") | test("::regulated_coin::Treasury$")) | .objectId' "$REG_FILE" | head -n1)
REGULATED_COIN_PACKAGE_ID=$(jq -r '.objectChanges[] | select(.type=="published" and (.modules | index("regulated_coin"))) | .packageId' "$REG_FILE")

if [[ -z "$REGULATED_COIN_ADMIN_CAP" || "$REGULATED_COIN_ADMIN_CAP" == "null" ]]; then
  echo "Could not find REGULATED_COIN_ADMIN_CAP in $REG_FILE" >&2; exit 1; fi
if [[ -z "$REGULATED_COIN_TREASURY" || "$REGULATED_COIN_TREASURY" == "null" ]]; then
  echo "Could not find REGULATED_COIN_TREASURY in $REG_FILE" >&2; exit 1; fi
if [[ -z "$REGULATED_COIN_PACKAGE_ID" || "$REGULATED_COIN_PACKAGE_ID" == "null" ]]; then
  echo "Could not find REGULATED_COIN_PACKAGE_ID in $REG_FILE" >&2; exit 1; fi

# Optional supply manager extraction
SUPPLY_MANAGER_PACKAGE_ID=""
SUPPLY_MANAGER_ADMIN_CAP=""
SUPPLY_MANAGER_OBJECT_ID=""
if [[ -n "$SUP_FILE" && -f "$SUP_FILE" ]]; then
  SUPPLY_MANAGER_PACKAGE_ID=$(jq -r '.objectChanges[] | select(.type=="published" and (.modules | index("supply_manager"))) | .packageId' "$SUP_FILE" | head -n1)
  SUPPLY_MANAGER_ADMIN_CAP=$(jq -r '.objectChanges[] | select((.objectType // "") | test("::supply_manager::SupplyManagerAdminCap$")) | .objectId' "$SUP_FILE" | head -n1)
  SUPPLY_MANAGER_OBJECT_ID=$(jq -r '.objectChanges[] | select((.objectType // "") | test("::supply_manager::SupplyManager($)")) | .objectId' "$SUP_FILE" 2>/dev/null | head -n1 || true)
fi

# Optional oft extraction
OFT_PACKAGE_ID=""
OFT_INIT_TICKET_ID=""
OFT_OAPP_ID=""
if [[ -n "$OFT_FILE" && -f "$OFT_FILE" ]]; then
  OFT_PACKAGE_ID=$(jq -r '.objectChanges[] | select(.type=="published" and (.modules | index("oft"))) | .packageId' "$OFT_FILE" | head -n1)
  OFT_INIT_TICKET_ID=$(jq -r '.objectChanges[] | select((.objectType // "") | test("::oft_impl::OFTInitTicket$")) | .objectId' "$OFT_FILE" | head -n1)
  OFT_OAPP_ID=$(jq -r '.objectChanges[] | select((.objectType // "") | test("::oapp::OApp$")) | .objectId' "$OFT_FILE" | head -n1)
fi

cat <<EOF
export REGULATED_COIN_ADMIN_CAP=$REGULATED_COIN_ADMIN_CAP
export REGULATED_COIN_TREASURY=$REGULATED_COIN_TREASURY
export REGULATED_COIN_PACKAGE_ID=$REGULATED_COIN_PACKAGE_ID
EOF

if [[ -n "$SUPPLY_MANAGER_PACKAGE_ID" ]]; then
  cat <<EOF
export SUPPLY_MANAGER_PACKAGE_ID=$SUPPLY_MANAGER_PACKAGE_ID
export SUPPLY_MANAGER_ADMIN_CAP=$SUPPLY_MANAGER_ADMIN_CAP
export SUPPLY_MANAGER_OBJECT_ID=$SUPPLY_MANAGER_OBJECT_ID
EOF
fi

if [[ -n "$OFT_PACKAGE_ID" ]]; then
  cat <<EOF
export OFT_PACKAGE_ID=$OFT_PACKAGE_ID
export OFT_INIT_TICKET_ID=$OFT_INIT_TICKET_ID
export OFT_OAPP_ID=$OFT_OAPP_ID
EOF
fi

# End of script
