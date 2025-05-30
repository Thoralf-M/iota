// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0
import { expect, test, type Page } from '@playwright/test';

import { faucet, split_coin } from './utils/localnet';

async function search(page: Page, text: string) {
    const searchbar = page.getByPlaceholder('Search');
    await searchbar.fill(text);
    const result = page.getByRole('button').getByText(text).first();
    await result.click();
}

test('can search for an address', async ({ page }) => {
    const address = await faucet();
    await page.goto('/');
    await search(page, address);
    await expect(page).toHaveURL(`/address/${address}`);
});

test('can search for objects', async ({ page }) => {
    const address = await faucet();
    const tx = await split_coin(address);

    const { objectId } = tx.effects!.created![0].reference;
    await page.goto('/');
    await search(page, objectId);
    await expect(page).toHaveURL(`/object/${objectId}`);
});

test('can search for transaction', async ({ page }) => {
    const address = await faucet();
    const tx = await split_coin(address);

    const txid = tx.digest;
    await page.goto('/');
    await search(page, txid);
    await expect(page).toHaveURL(`/txblock/${txid}`);
});
