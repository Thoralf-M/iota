// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import { createMessage } from '_messages';
import type { Message } from '_messages';
import type { PortChannelName } from '_src/shared/messaging/portChannelName';
import { isBasePayload, type ErrorPayload } from '_payloads';
import { isGetAccount } from '_src/shared/messaging/messages/payloads/account/getAccount';
import type { GetAccountResponse } from '_src/shared/messaging/messages/payloads/account/getAccountResponse';
import type { SetNetworkPayload } from '_payloads/network';
import {
    isAcquirePermissionsRequest,
    isHasPermissionRequest,
    type AcquirePermissionsResponse,
    type HasPermissionsResponse,
    type Permission,
    type PermissionType,
} from '_payloads/permissions';
import {
    isExecuteTransactionRequest,
    isSignTransactionRequest,
    type ExecuteTransactionResponse,
    type SignTransactionResponse,
} from '_payloads/transactions';
import Permissions from '_src/background/permissions';
import Transactions from '_src/background/transactions';
import {
    isSignMessageRequest,
    type SignMessageRequest,
} from '_src/shared/messaging/messages/payloads/transactions/signMessage';
import { type IotaTransactionBlockResponse } from '@iota/iota-sdk/client';
import type { Runtime } from 'webextension-polyfill';
import { type SignedTransaction } from '@iota/wallet-standard';

import { getAccountsStatusData } from '../accounts';
import NetworkEnv from '../networkEnv';
import { Connection } from './connection';

export class ContentScriptConnection extends Connection {
    public static readonly CHANNEL: PortChannelName = 'iota_content<->background';
    public readonly origin: string;
    public readonly pagelink?: string | undefined;
    public readonly originFavIcon: string | undefined;

    constructor(port: Runtime.Port) {
        super(port);
        this.origin = this.getOrigin(port);
        this.pagelink = this.getAppUrl(port);
        this.originFavIcon = port.sender?.tab?.favIconUrl;
    }

    protected async handleMessage(msg: Message) {
        const { payload } = msg;
        try {
            if (isGetAccount(payload)) {
                const { accounts } = await this.ensurePermissions(['viewAccount']);
                await this.sendAccounts(accounts, msg.id);
            } else if (isHasPermissionRequest(payload)) {
                this.send(
                    createMessage<HasPermissionsResponse>(
                        {
                            type: 'has-permissions-response',
                            result: await Permissions.hasPermissions(
                                this.origin,
                                payload.permissions,
                            ),
                        },
                        msg.id,
                    ),
                );
            } else if (isAcquirePermissionsRequest(payload)) {
                const permission = await Permissions.startRequestPermissions(
                    payload.permissions,
                    this,
                    msg.id,
                );
                if (permission) {
                    this.permissionReply(permission, msg.id);
                }
            } else if (isExecuteTransactionRequest(payload)) {
                if (!payload.transaction.account) {
                    // make sure we don't execute transactions that doesn't have a specified account
                    throw new Error('Missing account');
                }
                await this.ensurePermissions(
                    ['viewAccount', 'suggestTransactions'],
                    payload.transaction.account,
                );
                const result = await Transactions.executeOrSignTransaction(
                    { tx: payload.transaction },
                    this,
                );
                this.send(
                    createMessage<ExecuteTransactionResponse>(
                        {
                            type: 'execute-transaction-response',
                            result: result as IotaTransactionBlockResponse,
                        },
                        msg.id,
                    ),
                );
            } else if (isSignTransactionRequest(payload)) {
                if (!payload.transaction.account) {
                    // make sure we don't execute transactions that doesn't have a specified account
                    throw new Error('Missing account');
                }
                await this.ensurePermissions(
                    ['viewAccount', 'suggestTransactions'],
                    payload.transaction.account,
                );
                const result = await Transactions.executeOrSignTransaction(
                    { sign: payload.transaction },
                    this,
                );
                this.send(
                    createMessage<SignTransactionResponse>(
                        {
                            type: 'sign-transaction-response',
                            result: result as SignedTransaction,
                        },
                        msg.id,
                    ),
                );
            } else if (isBasePayload(payload) && payload.type === 'get-network') {
                this.send(
                    createMessage<SetNetworkPayload>(
                        {
                            type: 'set-network',
                            network: await NetworkEnv.getActiveNetwork(),
                        },
                        msg.id,
                    ),
                );
            } else if (isSignMessageRequest(payload) && payload.args) {
                await this.ensurePermissions(
                    ['viewAccount', 'suggestTransactions'],
                    payload.args.accountAddress,
                );
                const result = await Transactions.signPersonalMessage(payload.args, this);
                this.send(
                    createMessage<SignMessageRequest>(
                        { type: 'sign-personal-message-request', return: result },
                        msg.id,
                    ),
                );
            } else {
                throw new Error(`Unknown message, ${JSON.stringify(msg.payload)}`);
            }
        } catch (e) {
            this.sendError(
                {
                    error: true,
                    code: -1,
                    message: (e as Error).message,
                },
                msg.id,
            );
        }
    }

    public permissionReply(permission: Permission, msgID?: string) {
        if (permission.origin !== this.origin) {
            return;
        }
        const requestMsgID = msgID || permission.requestMsgID;
        if (permission.allowed) {
            this.send(
                createMessage<AcquirePermissionsResponse>(
                    {
                        type: 'acquire-permissions-response',
                        result: !!permission.allowed,
                    },
                    requestMsgID,
                ),
            );
        } else {
            this.sendError(
                {
                    error: true,
                    message: 'Permission rejected',
                    code: -1,
                },
                requestMsgID,
            );
        }
    }

    private getOrigin(port: Runtime.Port) {
        if (port.sender?.origin) {
            return port.sender.origin;
        }
        if (port.sender?.url) {
            return new URL(port.sender.url).origin;
        }
        throw new Error("[ContentScriptConnection] port doesn't include an origin");
    }

    // optional field for the app link.
    private getAppUrl(port: Runtime.Port) {
        if (port.sender?.url) {
            return new URL(port.sender.url).href;
        }
        return undefined;
    }

    private sendError<Error extends ErrorPayload>(error: Error, responseForID?: string) {
        this.send(createMessage(error, responseForID));
    }

    private async sendAccounts(accounts: string[], responseForID?: string) {
        this.send(
            createMessage<GetAccountResponse>(
                {
                    type: 'get-account-response',
                    accounts: await getAccountsStatusData(accounts),
                },
                responseForID,
            ),
        );
    }

    private async ensurePermissions(permissions: PermissionType[], account?: string) {
        const existingPermission = await Permissions.getPermission(this.origin);
        const allowed = await Permissions.hasPermissions(
            this.origin,
            permissions,
            existingPermission,
            account,
        );
        if (!allowed || !existingPermission) {
            throw new Error("Operation not allowed, dapp doesn't have the required permissions");
        }
        return existingPermission;
    }
}
