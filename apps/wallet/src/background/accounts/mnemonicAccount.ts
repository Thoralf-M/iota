// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import { fromExportedKeypair } from '_src/shared/utils';
import { type Keypair } from '@iota/iota-sdk/cryptography';

import { MnemonicAccountSource } from '../account-sources/mnemonicAccountSource';
import {
    Account,
    AccountType,
    type KeyPairExportableAccount,
    type PasswordUnlockableAccount,
    type SerializedAccount,
    type SerializedUIAccount,
    type SigningAccount,
} from './account';

export interface MnemonicSerializedAccount extends SerializedAccount {
    type: AccountType.MnemonicDerived;
    sourceID: string;
    derivationPath: string;
    publicKey: string;
}

export interface MnemonicSerializedUiAccount extends SerializedUIAccount {
    type: AccountType.MnemonicDerived;
    publicKey: string;
    derivationPath: string;
    sourceID: string;
}

export function isMnemonicSerializedUiAccount(
    account: SerializedUIAccount,
): account is MnemonicSerializedUiAccount {
    return account.type === AccountType.MnemonicDerived;
}

type SessionStorageData = { keyPair: string };

export class MnemonicAccount
    extends Account<MnemonicSerializedAccount, SessionStorageData>
    implements PasswordUnlockableAccount, SigningAccount, KeyPairExportableAccount
{
    readonly unlockType = 'password' as const;
    readonly canSign = true;
    readonly exportableKeyPair = true;

    static isOfType(serialized: SerializedAccount): serialized is MnemonicSerializedAccount {
        return serialized.type === AccountType.MnemonicDerived;
    }

    static createNew({
        keyPair,
        derivationPath,
        sourceID,
    }: {
        keyPair: Keypair;
        derivationPath: string;
        sourceID: string;
    }): Omit<MnemonicSerializedAccount, 'id'> {
        return {
            type: AccountType.MnemonicDerived,
            sourceID,
            address: keyPair.getPublicKey().toIotaAddress(),
            derivationPath,
            publicKey: keyPair.getPublicKey().toBase64(),
            lastUnlockedOn: null,
            selected: false,
            nickname: null,
            createdAt: Date.now(),
        };
    }

    constructor({ id, cachedData }: { id: string; cachedData?: MnemonicSerializedAccount }) {
        super({ type: AccountType.MnemonicDerived, id, cachedData });
    }

    async isLocked(): Promise<boolean> {
        return !(await this.#getKeyPair());
    }

    async lock(allowRead = false): Promise<void> {
        await this.clearEphemeralValue();
        await this.onLocked(allowRead);
    }

    async passwordUnlock(password?: string): Promise<void> {
        const mnemonicSource = await this.#getMnemonicSource();
        if ((await mnemonicSource.isLocked()) && !password) {
            throw new Error('Missing password to unlock the account');
        }
        const { derivationPath } = await this.getStoredData();
        if (password) {
            await mnemonicSource.unlock(password);
        }
        await this.setEphemeralValue({
            keyPair: (await mnemonicSource.deriveKeyPair(derivationPath)).getSecretKey(),
        });
        await this.onUnlocked();
    }

    async verifyPassword(password: string): Promise<void> {
        const mnemonicSource = await this.#getMnemonicSource();
        await mnemonicSource.verifyPassword(password);
    }

    async toUISerialized(): Promise<MnemonicSerializedUiAccount> {
        const { id, type, address, derivationPath, publicKey, sourceID, selected, nickname } =
            await this.getStoredData();
        return {
            id,
            type,
            address,
            isLocked: await this.isLocked(),
            derivationPath,
            publicKey,
            sourceID,
            lastUnlockedOn: await this.lastUnlockedOn,
            selected,
            nickname,
            isPasswordUnlockable: true,
            isKeyPairExportable: true,
        };
    }

    async signData(data: Uint8Array): Promise<string> {
        const keyPair = await this.#getKeyPair();
        if (!keyPair) {
            throw new Error(`Account is locked`);
        }
        return this.generateSignature(data, keyPair);
    }

    get derivationPath() {
        return this.getCachedData().then(({ derivationPath }) => derivationPath);
    }

    get sourceID() {
        return this.getCachedData().then(({ sourceID }) => sourceID);
    }

    async exportKeyPair(password: string): Promise<string> {
        const { derivationPath } = await this.getStoredData();
        const mnemonicSource = await this.#getMnemonicSource();
        await mnemonicSource.unlock(password);
        return (await mnemonicSource.deriveKeyPair(derivationPath)).getSecretKey();
    }

    async #getKeyPair() {
        const ephemeralData = await this.getEphemeralValue();
        if (ephemeralData) {
            return fromExportedKeypair(ephemeralData.keyPair);
        }
        return null;
    }

    async #getMnemonicSource() {
        return new MnemonicAccountSource((await this.getStoredData()).sourceID);
    }
}
