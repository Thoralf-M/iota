# Copyright (c) Mysten Labs, Inc.
# Modifications Copyright (c) 2025 IOTA Stiftung
# SPDX-License-Identifier: Apache-2.0

# test that we get an error if we supply both `--skip-dependency-verification` and `--verify-deps`

echo "=== publish (should fail) ===" | tee /dev/stderr
iota client --client.config $CONFIG publish example --skip-dependency-verification --verify-deps

echo "=== upgrade (should fail) ===" | tee /dev/stderr
iota client --client.config $CONFIG upgrade example --upgrade-capability 0x1234 --skip-dependency-verification --verify-deps
