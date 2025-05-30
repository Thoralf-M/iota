// Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import { type SerializedUIAccount } from '_src/background/accounts/account';
import {
    haveSupplyIncreaseLabel,
    COIN_TYPE,
    Collapsible,
    formatBalance,
    IOTA_COIN_METADATA,
    STARDUST_BASIC_OUTPUT_TYPE,
    STARDUST_NFT_OUTPUT_TYPE,
    TIMELOCK_IOTA_TYPE,
    TIMELOCK_STAKED_TYPE,
    useBalance,
    useFormatCoin,
} from '@iota/core';
import { Info, TriangleDown } from '@iota/apps-ui-icons';
import clsx from 'clsx';
import { Badge, BadgeType } from '@iota/apps-ui-kit';
import { formatAddress, IOTA_TYPE_ARG } from '@iota/iota-sdk/utils';
import {
    useGetOwnedObjectsMultipleAddresses,
    useGetSharedObjectsMultipleAddresses,
} from '../../hooks';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useIotaClientContext } from '@iota/dapp-kit';
import { useEffect, useState } from 'react';

interface AccountBalanceItemProps {
    accounts: SerializedUIAccount[];
    accountIndex: string;
}

const OBJECT_PER_REQ = 1;

export function AccountBalanceItem({
    accounts,
    accountIndex,
}: AccountBalanceItemProps): JSX.Element {
    const [hasVestingObjects, setHasVestingObjects] = useState<boolean>(false);

    const queryClient = useQueryClient();
    const iotaContext = useIotaClientContext();

    const addresses = accounts.map(({ address }) => address);

    const { data: sumOfBalances } = useQuery({
        queryKey: ['getBalance', ...addresses],
        async queryFn() {
            return await Promise.all(
                addresses.map(async (address) => {
                    const params = {
                        coinType: IOTA_TYPE_ARG,
                        owner: address!,
                    };
                    return queryClient.ensureQueryData({
                        queryKey: [iotaContext.network, 'getBalance', params],
                        queryFn: () => iotaContext.client.getBalance(params),
                    });
                }),
            );
        },
        select(balances) {
            const balance = balances.reduce((acc, { totalBalance }) => {
                return BigInt(acc) + BigInt(totalBalance);
            }, BigInt(0));
            const formattedAmount = formatBalance(balance, IOTA_COIN_METADATA.decimals);
            return `${formattedAmount} ${IOTA_COIN_METADATA.symbol}`;
        },
        gcTime: 0,
        staleTime: 0,
    });

    const { data: ownedObjects } = useGetOwnedObjectsMultipleAddresses(
        addresses,
        {
            MatchNone: [
                { StructType: COIN_TYPE },
                { StructType: TIMELOCK_IOTA_TYPE },
                { StructType: TIMELOCK_STAKED_TYPE },
                { StructType: STARDUST_BASIC_OUTPUT_TYPE },
                { StructType: STARDUST_NFT_OUTPUT_TYPE },
            ],
        },
        OBJECT_PER_REQ,
    );

    const { data: stardustOwnedObjects } = useGetOwnedObjectsMultipleAddresses(
        addresses,
        {
            MatchAny: [
                { StructType: STARDUST_BASIC_OUTPUT_TYPE },
                { StructType: STARDUST_NFT_OUTPUT_TYPE },
            ],
        },
        OBJECT_PER_REQ,
    );

    const { data: stardustSharedObjects } = useGetSharedObjectsMultipleAddresses(
        addresses,
        OBJECT_PER_REQ,
    );

    const hasMigrationObjects =
        stardustOwnedObjects?.pages?.some((data) => data.some((data) => data.data.length > 0)) ||
        stardustSharedObjects?.pages?.some((data) =>
            data.some((data) => data.nftOutputs.length > 0 || data.basicOutputs.length > 0),
        );

    const hasAccountAssets = !!ownedObjects?.pages.some((data) =>
        data.some((data) => data.data.length > 0),
    );

    const {
        data: vestingObjects,
        hasNextPage,
        fetchNextPage,
    } = useGetOwnedObjectsMultipleAddresses(
        addresses,
        {
            MatchAny: [{ StructType: TIMELOCK_IOTA_TYPE }, { StructType: TIMELOCK_STAKED_TYPE }],
        },
        10,
    );

    useEffect(() => {
        if (vestingObjects?.pages) {
            const foundVestingObject = haveSupplyIncreaseLabel(vestingObjects.pages.flat());
            setHasVestingObjects(foundVestingObject);

            if (!foundVestingObject && hasNextPage) {
                fetchNextPage();
            }
        }
    }, [vestingObjects, hasNextPage]);

    return (
        <Collapsible
            defaultOpen
            hideArrow
            render={({ isOpen }) => (
                <div className="relative flex min-h-[52px] w-full items-center justify-between gap-1 py-2 pl-1 pr-sm text-neutral-10 dark:text-neutral-92">
                    <div className="flex items-center gap-xxs">
                        <TriangleDown
                            className={clsx(
                                'h-5 w-5 ',
                                isOpen
                                    ? 'rotate-0 transition-transform ease-linear'
                                    : '-rotate-90 transition-transform ease-linear',
                            )}
                        />
                        <div className="flex flex-col items-start gap-xxs">
                            <div className="text-title-md">Wallet {Number(accountIndex) + 1}</div>
                            <span className="text-body-sm text-neutral-40 dark:text-neutral-60">
                                {accounts.length} {accounts.length > 1 ? 'addresses' : 'address'}
                            </span>
                        </div>
                    </div>
                    <div className="flex flex-col items-end gap-xxs">
                        <span>{sumOfBalances}</span>
                        <div className="flex flex-row gap-xxs">
                            {hasAccountAssets && <Badge type={BadgeType.Neutral} label="Assets" />}
                            {hasVestingObjects && (
                                <Badge type={BadgeType.Neutral} label="Vesting" />
                            )}
                            {hasMigrationObjects && (
                                <Badge type={BadgeType.Neutral} label="Migration" />
                            )}
                        </div>
                    </div>
                </div>
            )}
        >
            {(hasMigrationObjects || hasVestingObjects) && (
                <div className="mx-sm flex flex-row items-start gap-x-xs rounded-xl border border-shader-neutral-light-8 p-xs text-neutral-10 dark:border-shader-neutral-dark-8 dark:text-neutral-92">
                    <Info className="h-4 w-4 shrink-0" />
                    <small>
                        For an accurate portfolio view, ensure your total balance matches the total
                        of your found IOTA, vested, and migration funds. Check the Dashboard tabs
                        for details
                    </small>
                </div>
            )}
            <div className="flex flex-col gap-y-sm p-sm pl-lg text-body-md text-neutral-10 dark:text-neutral-92">
                {accounts.map(({ address, id }) => (
                    <AddressItem key={id} address={address} />
                ))}
            </div>
        </Collapsible>
    );
}

export function AddressItem({ address }: { address: string }): JSX.Element {
    const { data: balance } = useBalance(address!);
    const totalBalance = balance?.totalBalance || '0';
    const coinType = balance?.coinType || '';
    const [formatted, symbol] = useFormatCoin({ balance: BigInt(totalBalance), coinType });

    return (
        <div className="flex w-full flex-row justify-between">
            <span>{formatAddress(address)}</span>
            <span>{`${formatted} ${symbol}`}</span>
        </div>
    );
}
