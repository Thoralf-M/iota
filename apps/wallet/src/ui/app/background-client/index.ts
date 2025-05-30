// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import { createMessage } from '_messages';
import type { Message } from '_messages';
import { PortStream } from '_src/shared/messaging/portStream';
import { type BasePayload } from '_payloads';
import { isLoadedFeaturesPayload } from '_payloads/feature-gating';
import { isSetNetworkPayload, type SetNetworkPayload } from '_payloads/network';
import { isPermissionRequests } from '_payloads/permissions';
import type { GetPermissionRequests, PermissionResponse } from '_payloads/permissions';
import type { DisconnectApp } from '_src/shared/messaging/messages/payloads/permissions/disconnectApp';
import { isUpdateActiveOrigin } from '_payloads/tabs/updateActiveOrigin';
import type { GetTransactionRequests } from '_src/shared/messaging/messages/payloads/transactions/ui/getTransactionRequests';
import { isGetTransactionRequestsResponse } from '_src/shared/messaging/messages/payloads/transactions/ui/getTransactionRequestsResponse';
import type { TransactionRequestResponse } from '_src/shared/messaging/messages/payloads/transactions/ui/transactionRequestResponse';
import { changeActiveNetwork, setActiveOrigin } from '_redux/slices/app';
import { setPermissions } from '_redux/slices/permissions';
import { setTransactionRequests } from '_redux/slices/transaction-requests';
import { type MnemonicSerializedUiAccount } from '_src/background/accounts/mnemonicAccount';
import { type SeedSerializedUiAccount } from '_src/background/accounts/seedAccount';
import type { NetworkEnvType } from '@iota/core';
import {
    isMethodPayload,
    type MethodPayload,
    type UIAccessibleEntityType,
} from '_src/shared/messaging/messages/payloads/methodPayload';
import { type SignedMessage, type SignedTransaction } from '_src/ui/app/walletSigner';
import type { AppDispatch } from '_store';
import { type IotaTransactionBlockResponse } from '@iota/iota-sdk/client';
import { toB64 } from '@iota/iota-sdk/utils';
import { type QueryKey } from '@tanstack/react-query';
import { lastValueFrom, map, take } from 'rxjs';
import { growthbook } from '../experimentation/featureGating';
import { ACCOUNTS_QUERY_KEY } from '../helpers/queryClientKeys';
import { queryClient } from '../helpers/queryClient';
import { ACCOUNT_SOURCES_QUERY_KEY } from '../hooks/useAccountSources';
import { AccountSourceType } from '_src/background/account-sources/accountSource';
import {
    type DeriveBipPathAccountsFinder,
    isDeriveBipPathAccountsFinderResponse,
    type PersistAccountsFinder,
    type SourceStrategyToPersist,
} from '_src/shared/messaging/messages/payloads/accounts-finder';
import { type MakeDerivationOptions } from '_src/background/account-sources/bip44Path';

const ENTITIES_TO_CLIENT_QUERY_KEYS: Record<UIAccessibleEntityType, QueryKey> = {
    accounts: ACCOUNTS_QUERY_KEY,
    accountSources: ACCOUNT_SOURCES_QUERY_KEY,
};

export class BackgroundClient {
    private _portStream: PortStream | null = null;
    private _dispatch: AppDispatch | null = null;
    private _initialized = false;

    public init(dispatch: AppDispatch) {
        if (this._initialized) {
            throw new Error('[BackgroundClient] already initialized');
        }
        this._initialized = true;
        this._dispatch = dispatch;
        this.createPortStream();
        return Promise.all([
            this.sendGetPermissionRequests(),
            this.sendGetTransactionRequests(),
            this.loadFeatures(),
            this.getNetwork(),
        ]).then(() => undefined);
    }

    public sendPermissionResponse(
        id: string,
        accounts: string[],
        allowed: boolean,
        responseDate: string,
    ) {
        this.sendMessage(
            createMessage<PermissionResponse>({
                id,
                type: 'permission-response',
                accounts,
                allowed,
                responseDate,
            }),
        );
    }

    public sendGetPermissionRequests() {
        return lastValueFrom(
            this.sendMessage(
                createMessage<GetPermissionRequests>({
                    type: 'get-permission-requests',
                }),
            ).pipe(take(1)),
        );
    }

    public sendTransactionRequestResponse(
        txID: string,
        approved: boolean,
        txResult?: IotaTransactionBlockResponse | SignedMessage,
        txResultError?: string,
        txSigned?: SignedTransaction,
    ) {
        this.sendMessage(
            createMessage<TransactionRequestResponse>({
                type: 'transaction-request-response',
                approved,
                txID,
                txResult,
                txResultError,
                txSigned,
            }),
        );
    }

    public sendGetTransactionRequests() {
        return lastValueFrom(
            this.sendMessage(
                createMessage<GetTransactionRequests>({
                    type: 'get-transaction-requests',
                }),
            ).pipe(take(1)),
        );
    }

    /**
     * Disconnect a dapp, if specificAccounts contains accounts then only those accounts will be disconnected.
     * @param origin The origin of the dapp
     * @param specificAccounts Accounts to disconnect. If not provided or it's an empty array all accounts will be disconnected
     */
    public async disconnectApp(origin: string, specificAccounts?: string[]) {
        await lastValueFrom(
            this.sendMessage(
                createMessage<DisconnectApp>({
                    type: 'disconnect-app',
                    origin,
                    specificAccounts,
                }),
            ).pipe(take(1)),
        );
    }

    public clearWallet() {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'clearWallet'>>({
                    type: 'method-payload',
                    method: 'clearWallet',
                    args: {},
                }),
            ).pipe(take(1)),
        );
    }

    public signData(addressOrID: string, data: Uint8Array): Promise<string> {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'signData'>>({
                    type: 'method-payload',
                    method: 'signData',
                    args: { data: toB64(data), id: addressOrID },
                }),
            ).pipe(
                take(1),
                map(({ payload }) => {
                    if (isMethodPayload(payload, 'signDataResponse')) {
                        return payload.args.signature;
                    }
                    throw new Error('Error unknown response for signData message');
                }),
            ),
        );
    }

    public setActiveNetworkEnv(network: NetworkEnvType) {
        return lastValueFrom(
            this.sendMessage(
                createMessage<SetNetworkPayload>({
                    type: 'set-network',
                    network,
                }),
            ).pipe(take(1)),
        );
    }

    public selectAccount(accountID: string) {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'switchAccount'>>({
                    type: 'method-payload',
                    method: 'switchAccount',
                    args: { accountID },
                }),
            ).pipe(take(1)),
        );
    }

    public verifyPassword(args: MethodPayload<'verifyPassword'>['args']) {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'verifyPassword'>>({
                    type: 'method-payload',
                    method: 'verifyPassword',
                    args,
                }),
            ).pipe(take(1)),
        );
    }

    public exportAccountKeyPair(args: MethodPayload<'getAccountKeyPair'>['args']) {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'getAccountKeyPair'>>({
                    type: 'method-payload',
                    method: 'getAccountKeyPair',
                    args,
                }),
            ).pipe(
                take(1),
                map(({ payload }) => {
                    if (isMethodPayload(payload, 'getAccountKeyPairResponse')) {
                        return payload.args;
                    }
                    throw new Error('Error unknown response for export account message');
                }),
            ),
        );
    }

    public getStoredEntities<R>(type: UIAccessibleEntityType): Promise<R[]> {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'getStoredEntities'>>({
                    method: 'getStoredEntities',
                    type: 'method-payload',
                    args: { type },
                }),
            ).pipe(
                take(1),
                map(({ payload }) => {
                    if (!isMethodPayload(payload, 'storedEntitiesResponse')) {
                        throw new Error('Unknown response');
                    }
                    if (type !== payload.args.type) {
                        throw new Error(`unexpected entity type response ${payload.args.type}`);
                    }
                    return payload.args.entities as R[];
                }),
            ),
        );
    }

    public createMnemonicAccountSource(inputs: { password: string; entropy?: string }) {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'createAccountSource'>>({
                    method: 'createAccountSource',
                    type: 'method-payload',
                    args: { type: AccountSourceType.Mnemonic, params: inputs },
                }),
            ).pipe(
                take(1),
                map(({ payload }) => {
                    if (!isMethodPayload(payload, 'accountSourceCreationResponse')) {
                        throw new Error('Unknown response');
                    }
                    if (AccountSourceType.Mnemonic !== payload.args.accountSource.type) {
                        throw new Error(
                            `Unexpected account source type response ${payload.args.accountSource.type}`,
                        );
                    }
                    return payload.args.accountSource as unknown as MnemonicSerializedUiAccount;
                }),
            ),
        );
    }

    public createSeedAccountSource(inputs: { password: string; seed: string }) {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'createAccountSource'>>({
                    method: 'createAccountSource',
                    type: 'method-payload',
                    args: { type: AccountSourceType.Seed, params: inputs },
                }),
            ).pipe(
                take(1),
                map(({ payload }) => {
                    if (!isMethodPayload(payload, 'accountSourceCreationResponse')) {
                        throw new Error('Unknown response');
                    }
                    if (AccountSourceType.Seed !== payload.args.accountSource.type) {
                        throw new Error(
                            `Unexpected account source type response ${payload.args.accountSource.type}`,
                        );
                    }
                    return payload.args.accountSource as unknown as SeedSerializedUiAccount;
                }),
            ),
        );
    }

    public createAccounts(inputs: MethodPayload<'createAccounts'>['args']) {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'createAccounts'>>({
                    method: 'createAccounts',
                    type: 'method-payload',
                    args: inputs,
                }),
            ).pipe(
                take(1),
                map(({ payload }) => {
                    if (!isMethodPayload(payload, 'accountsCreatedResponse')) {
                        throw new Error('Unknown response');
                    }
                    if (inputs.type !== payload.args.accounts[0]?.type) {
                        throw new Error(
                            `Unexpected accounts type response ${payload.args.accounts[0]?.type}`,
                        );
                    }
                    return payload.args.accounts;
                }),
            ),
        );
    }

    public unlockAccountSourceOrAccount(
        inputs: MethodPayload<'unlockAccountSourceOrAccount'>['args'],
    ) {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'unlockAccountSourceOrAccount'>>({
                    type: 'method-payload',
                    method: 'unlockAccountSourceOrAccount',
                    args: inputs,
                }),
            ).pipe(take(1)),
        );
    }

    public lockAccountSourceOrAccount({ id }: MethodPayload<'lockAccountSourceOrAccount'>['args']) {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'lockAccountSourceOrAccount'>>({
                    type: 'method-payload',
                    method: 'lockAccountSourceOrAccount',
                    args: { id },
                }),
            ).pipe(take(1)),
        );
    }

    public setAccountNickname({ id, nickname }: MethodPayload<'setAccountNickname'>['args']) {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'setAccountNickname'>>({
                    type: 'method-payload',
                    method: 'setAccountNickname',
                    args: { id, nickname },
                }),
            ).pipe(take(1)),
        );
    }

    public getStorageMigrationStatus() {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'getStorageMigrationStatus'>>({
                    method: 'getStorageMigrationStatus',
                    type: 'method-payload',
                    args: null,
                }),
            ).pipe(
                take(1),
                map(({ payload }) => {
                    if (!isMethodPayload(payload, 'storageMigrationStatus')) {
                        throw new Error('Unknown response');
                    }
                    return payload.args.status;
                }),
            ),
        );
    }

    public doStorageMigration(inputs: MethodPayload<'doStorageMigration'>['args']) {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'doStorageMigration'>>({
                    type: 'method-payload',
                    method: 'doStorageMigration',
                    args: inputs,
                }),
            ).pipe(take(1)),
        );
    }

    /**
     * Wallet wasn't storing the public key of ledger accounts, but we need it to send it to the dapps.
     * Use this function to update the public keys whenever wallet has access to them.
     */
    public storeLedgerAccountsPublicKeys(
        args: MethodPayload<'storeLedgerAccountsPublicKeys'>['args'],
    ) {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'storeLedgerAccountsPublicKeys'>>({
                    type: 'method-payload',
                    method: 'storeLedgerAccountsPublicKeys',
                    args,
                }),
            ).pipe(take(1)),
        );
    }

    public getAccountSourceEntropy(args: MethodPayload<'getAccountSourceEntropy'>['args']) {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'getAccountSourceEntropy'>>({
                    type: 'method-payload',
                    method: 'getAccountSourceEntropy',
                    args,
                }),
            ).pipe(
                take(1),
                map(({ payload }) => {
                    if (isMethodPayload(payload, 'getAccountSourceEntropyResponse')) {
                        return payload.args;
                    }
                    throw new Error('Unexpected response type');
                }),
            ),
        );
    }

    public getAccountSourceSeed(args: MethodPayload<'getAccountSourceSeed'>['args']) {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'getAccountSourceSeed'>>({
                    type: 'method-payload',
                    method: 'getAccountSourceSeed',
                    args,
                }),
            ).pipe(
                take(1),
                map(({ payload }) => {
                    if (isMethodPayload(payload, 'getAccountSourceSeedResponse')) {
                        return payload.args;
                    }
                    throw new Error('Unexpected response type');
                }),
            ),
        );
    }

    public getAutoLockMinutes() {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'getAutoLockMinutes'>>({
                    type: 'method-payload',
                    method: 'getAutoLockMinutes',
                    args: {},
                }),
            ).pipe(
                take(1),
                map(({ payload }) => {
                    if (isMethodPayload(payload, 'getAutoLockMinutesResponse')) {
                        return payload.args.minutes;
                    }
                    throw new Error('Unexpected response type');
                }),
            ),
        );
    }

    public setAutoLockMinutes(args: MethodPayload<'setAutoLockMinutes'>['args']) {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'setAutoLockMinutes'>>({
                    type: 'method-payload',
                    method: 'setAutoLockMinutes',
                    args,
                }),
            ).pipe(take(1)),
        );
    }

    public notifyUserActive() {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'notifyUserActive'>>({
                    type: 'method-payload',
                    method: 'notifyUserActive',
                    args: {},
                }),
            ).pipe(take(1)),
        );
    }

    public resetPassword(args: MethodPayload<'resetPassword'>['args']) {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'resetPassword'>>({
                    type: 'method-payload',
                    method: 'resetPassword',
                    args,
                }),
            ).pipe(take(1)),
        );
    }

    public verifyPasswordRecoveryData(args: MethodPayload<'verifyPasswordRecoveryData'>['args']) {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'verifyPasswordRecoveryData'>>({
                    type: 'method-payload',
                    method: 'verifyPasswordRecoveryData',
                    args,
                }),
            ).pipe(take(1)),
        );
    }

    public removeAccount(args: MethodPayload<'removeAccount'>['args']) {
        return lastValueFrom(
            this.sendMessage(
                createMessage<MethodPayload<'removeAccount'>>({
                    type: 'method-payload',
                    method: 'removeAccount',
                    args,
                }),
            ).pipe(take(1)),
        );
    }

    public deriveBipPathAccountsFinder(sourceID: string, derivationOptions: MakeDerivationOptions) {
        return lastValueFrom(
            this.sendMessage(
                createMessage<DeriveBipPathAccountsFinder>({
                    type: 'derive-bip-path-accounts-finder',
                    sourceID,
                    derivationOptions,
                }),
            ).pipe(
                take(1),
                map(({ payload }) => {
                    if (isDeriveBipPathAccountsFinderResponse(payload)) {
                        return payload;
                    }
                    throw new Error('Unexpected response type');
                }),
            ),
        );
    }

    public async persistAccountsFinder(sourceStrategy: SourceStrategyToPersist) {
        await lastValueFrom(
            this.sendMessage(
                createMessage<PersistAccountsFinder>({
                    type: 'persist-accounts-finder',
                    sourceStrategy,
                }),
            ).pipe(take(1)),
        );
    }

    private loadFeatures() {
        return lastValueFrom(
            this.sendMessage(
                createMessage<BasePayload>({
                    type: 'get-features',
                }),
            ).pipe(take(1)),
        );
    }

    private getNetwork() {
        return lastValueFrom(
            this.sendMessage(
                createMessage<BasePayload>({
                    type: 'get-network',
                }),
            ).pipe(take(1)),
        );
    }

    private handleIncomingMessage(msg: Message) {
        if (!this._initialized || !this._dispatch) {
            throw new Error('BackgroundClient is not initialized to handle incoming messages');
        }
        const { payload } = msg;
        let action;
        if (isPermissionRequests(payload)) {
            action = setPermissions(payload.permissions);
        } else if (isGetTransactionRequestsResponse(payload)) {
            action = setTransactionRequests(payload.txRequests);
        } else if (isUpdateActiveOrigin(payload)) {
            action = setActiveOrigin(payload);
        } else if (isLoadedFeaturesPayload(payload)) {
            growthbook.setAttributes(payload.attributes);
            growthbook.setFeatures(payload.features);
        } else if (isSetNetworkPayload(payload)) {
            action = changeActiveNetwork({
                network: payload.network,
            });
        } else if (isMethodPayload(payload, 'entitiesUpdated')) {
            const entitiesQueryKey = ENTITIES_TO_CLIENT_QUERY_KEYS[payload.args.type];
            if (entitiesQueryKey) {
                queryClient.invalidateQueries({ queryKey: entitiesQueryKey });
            }
        }
        if (action) {
            this._dispatch(action);
        }
    }

    private createPortStream() {
        this._portStream = PortStream.connectToBackgroundService('iota_ui<->background');
        this._portStream.onDisconnect.subscribe(() => {
            this.createPortStream();
        });
        this._portStream.onMessage.subscribe((msg) => this.handleIncomingMessage(msg));
    }

    private sendMessage(msg: Message) {
        if (this._portStream?.connected) {
            return this._portStream.sendMessage(msg);
        } else {
            throw new Error('Failed to send message to background service. Port not connected.');
        }
    }
}
