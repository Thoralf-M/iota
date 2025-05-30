// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import { bcs } from '@iota/iota-sdk/bcs';
import { getFullnodeUrl, IotaClient } from '@iota/iota-sdk/client';
import { Transaction } from '@iota/iota-sdk/transactions';
import { act, renderHook, waitFor } from '@testing-library/react';
import { expect, type Mock } from 'vitest';

import { toB58 } from '../../../bcs/dist/cjs/b58.js';
import {
    WalletFeatureNotSupportedError,
    WalletNotConnectedError,
} from '../../src/errors/walletErrors.js';
import { useConnectWallet, useSignAndExecuteTransaction } from '../../src/index.js';
import { iotaFeatures } from '../mocks/mockFeatures.js';
import { createWalletProviderContextWrapper, registerMockWallet } from '../test-utils.js';

describe('useSignAndExecuteTransaction', () => {
    test('throws an error when trying to sign and execute a transaction without a wallet connection', async () => {
        const wrapper = createWalletProviderContextWrapper();
        const { result } = renderHook(() => useSignAndExecuteTransaction(), { wrapper });

        result.current.mutate({ transaction: new Transaction(), chain: 'iota:testnet' });

        await waitFor(() => expect(result.current.error).toBeInstanceOf(WalletNotConnectedError));
    });

    test('throws an error when trying to sign and execute a transaction with a wallet that lacks feature support', async () => {
        const { unregister, mockWallet } = registerMockWallet({
            walletName: 'Mock Wallet 1',
        });

        const wrapper = createWalletProviderContextWrapper();
        const { result } = renderHook(
            () => ({
                connectWallet: useConnectWallet(),
                useSignAndExecuteTransaction: useSignAndExecuteTransaction(),
            }),
            { wrapper },
        );

        result.current.connectWallet.mutate({ wallet: mockWallet });
        await waitFor(() => expect(result.current.connectWallet.isSuccess).toBe(true));

        result.current.useSignAndExecuteTransaction.mutate({
            transaction: new Transaction(),
            chain: 'iota:testnet',
        });
        await waitFor(() =>
            expect(result.current.useSignAndExecuteTransaction.error).toBeInstanceOf(
                WalletFeatureNotSupportedError,
            ),
        );

        act(() => unregister());
    });

    test('signing and executing a transaction from the currently connected account works successfully', async () => {
        const { unregister, mockWallet } = registerMockWallet({
            walletName: 'Mock Wallet 1',
            features: iotaFeatures,
        });

        const iotaClient = new IotaClient({ url: getFullnodeUrl('localnet') });
        const mockSignMessageFeature = mockWallet.features['iota:signTransaction'];
        const signTransaction = mockSignMessageFeature!.signTransaction as Mock;

        signTransaction.mockReturnValueOnce({
            bytes: 'abc',
            signature: '123',
        });

        const reportEffectsFeature = mockWallet.features['iota:reportTransactionEffects'];
        const reportEffects = reportEffectsFeature!.reportTransactionEffects as Mock;

        reportEffects.mockImplementation(async () => {});

        const executeTransaction = vi.spyOn(iotaClient, 'executeTransactionBlock');

        executeTransaction.mockResolvedValueOnce({
            digest: '123',
            rawEffects: [10, 20, 30],
        });

        const wrapper = createWalletProviderContextWrapper({}, iotaClient);
        const { result } = renderHook(
            () => ({
                connectWallet: useConnectWallet(),
                useSignAndExecuteTransaction: useSignAndExecuteTransaction(),
            }),
            { wrapper },
        );

        result.current.connectWallet.mutate({ wallet: mockWallet });

        await waitFor(() => expect(result.current.connectWallet.isSuccess).toBe(true));

        const signTransactionFeature = mockWallet.features['iota:signTransaction'];
        const signTransactionMock = signTransactionFeature!.signTransaction as Mock;

        signTransactionMock.mockReturnValueOnce({
            transactionBytes: 'abc',
            signature: '123',
        });

        result.current.useSignAndExecuteTransaction.mutate({
            transaction: new Transaction(),
            chain: 'iota:testnet',
        });

        await waitFor(() =>
            expect(result.current.useSignAndExecuteTransaction.isSuccess).toBe(true),
        );
        expect(result.current.useSignAndExecuteTransaction.data).toStrictEqual({
            bytes: 'abc',
            digest: '123',
            effects: 'ChQe',
            signature: '123',
            rawEffects: [10, 20, 30],
        });
        expect(reportEffects).toHaveBeenCalledWith({
            effects: 'ChQe',
            chain: 'iota:testnet',
            account: mockWallet.accounts[0],
        });

        const call = signTransaction.mock.calls[0];

        expect(call[0].account).toStrictEqual(mockWallet.accounts[0]);
        expect(call[0].chain).toBe('iota:testnet');
        expect(await call[0].transaction.toJSON()).toEqual(await new Transaction().toJSON());

        act(() => unregister());
    });

    test('executing with custom data resolver', async () => {
        const { unregister, mockWallet } = registerMockWallet({
            walletName: 'Mock Wallet 1',
            features: iotaFeatures,
        });

        const iotaClient = new IotaClient({ url: getFullnodeUrl('localnet') });
        const mockSignMessageFeature = mockWallet.features['iota:signTransaction'];
        const signTransaction = mockSignMessageFeature!.signTransaction as Mock;

        signTransaction.mockReturnValueOnce({
            bytes: 'abc',
            signature: '123',
        });

        const reportEffectsFeature = mockWallet.features['iota:reportTransactionEffects'];
        const reportEffects = reportEffectsFeature!.reportTransactionEffects as Mock;

        reportEffects.mockImplementation(async () => {});

        const wrapper = createWalletProviderContextWrapper({}, iotaClient);

        const fakeDigest = toB58(
            new Uint8Array([
                1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 1, 2, 3, 4, 5, 6, 7, 8,
                9, 0, 1, 2,
            ]),
        );
        const effectsBcs = bcs.TransactionEffects.serialize({
            V1: {
                status: {
                    Success: true,
                },
                executedEpoch: 1,
                gasUsed: {
                    computationCost: 1,
                    computationCostBurned: 1,
                    storageCost: 1,
                    storageRebate: 1,
                    nonRefundableStorageFee: 1,
                },
                transactionDigest: fakeDigest,
                gasObjectIndex: 0,
                eventsDigest: fakeDigest,
                dependencies: [],
                lamportVersion: 1,
                changedObjects: [],
                unchangedSharedObjects: [],
                auxDataDigest: fakeDigest,
            },
        }).toBase64();
        const { result } = renderHook(
            () => ({
                connectWallet: useConnectWallet(),
                useSignAndExecuteTransaction: useSignAndExecuteTransaction({
                    execute: async () => ({
                        custom: 123,
                        effects: {
                            bcs: effectsBcs,
                        },
                    }),
                }),
            }),
            { wrapper },
        );

        result.current.connectWallet.mutate({ wallet: mockWallet });

        await waitFor(() => expect(result.current.connectWallet.isSuccess).toBe(true));

        const signTransactionFeature = mockWallet.features['iota:signTransaction'];
        const signTransactionMock = signTransactionFeature!.signTransaction as Mock;

        signTransactionMock.mockReturnValueOnce({
            transactionBytes: 'abc',
            signature: '123',
        });

        result.current.useSignAndExecuteTransaction.mutate({
            transaction: new Transaction(),
            chain: 'iota:testnet',
        });

        await waitFor(() =>
            expect(result.current.useSignAndExecuteTransaction.isSuccess).toBe(true),
        );
        expect(result.current.useSignAndExecuteTransaction.data).toStrictEqual({
            effects: {
                bcs: effectsBcs,
            },
            custom: 123,
        });
        expect(result.current.useSignAndExecuteTransaction.data?.custom).toBe(123);
        expect(reportEffects).toHaveBeenCalledWith({
            account: mockWallet.accounts[0],
            chain: 'iota:testnet',
            effects: effectsBcs,
        });

        const call = signTransaction.mock.calls[0];

        expect(call[0].account).toStrictEqual(mockWallet.accounts[0]);
        expect(call[0].chain).toBe('iota:testnet');
        expect(await call[0].transaction.toJSON()).toEqual(await new Transaction().toJSON());

        act(() => unregister());
    });
});
