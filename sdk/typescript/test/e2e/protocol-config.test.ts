// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import { expect, it } from 'vitest';

import { setup } from './utils/setup';

it('can fetch protocol config', async () => {
    const toolbox = await setup();
    const config = await toolbox.client.getProtocolConfig();
    expect(config).toBeTypeOf('object');
});
