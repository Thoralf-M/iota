// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import {
    type KeyDerivationOptions,
    decrypt as metamaskDecrypt,
    encrypt as metamaskEncrypt,
} from '@metamask/browser-passworder';
import { randomBytes } from '@noble/hashes/utils';

// we use this password + a random one for each time we store the encrypted
// vault to session storage
const PASSWORD =
    process.env.WALLET_KEYRING_PASSWORD ||
    '344c6f7d04a65c24f35f5c710b0e91e2f2e2f88c038562622d5602019b937bc2c2aa2821e65cc94775fe5acf2fee240d38f1abbbe00b0e6682646a4ce10e908e';

const KD_OPTIONS: KeyDerivationOptions = {
    algorithm: 'PBKDF2',
    params: {
        iterations: 150_000,
    },
};

export type Serializable =
    | string
    | number
    | boolean
    | null
    | { [index: string]: Serializable | undefined }
    | Serializable[]
    | (Iterable<Serializable> & { length: number });

export async function encrypt(password: string, secrets: Serializable): Promise<string> {
    return metamaskEncrypt(password, secrets, undefined, undefined, KD_OPTIONS);
}

export async function decrypt<T extends Serializable>(
    password: string,
    ciphertext: string,
): Promise<T> {
    return (await metamaskDecrypt(password, ciphertext)) as T;
}

export function getRandomPassword() {
    return Buffer.from(randomBytes(64)).toString('hex');
}

export function makeEphemeraPassword(rndPass: string) {
    return `${PASSWORD}${rndPass}`;
}
