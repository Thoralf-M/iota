// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import { ExplorerLinkType, Loading, UnlockAccountButton } from '_components';
import { useActiveAccount, useActiveAddress, useAppSelector, useExplorerLink } from '_hooks';
import { FaucetRequestButton } from '_src/ui/app/shared/faucet/FaucetRequestButton';
import { useFeature } from '@growthbook/growthbook-react';
import {
    Feature,
    DELEGATED_STAKES_QUERY_REFETCH_INTERVAL,
    DELEGATED_STAKES_QUERY_STALE_TIME,
    useAppsBackend,
    useGetDelegatedStake,
    TIMELOCK_IOTA_TYPE,
    useGetOwnedObjects,
    TIMELOCK_STAKED_TYPE,
    STARDUST_BASIC_OUTPUT_TYPE,
    STARDUST_NFT_OUTPUT_TYPE,
    useGetStardustSharedBasicObjects,
    useGetStardustSharedNftObjects,
    useGetAllBalances,
    toast,
    haveSupplyIncreaseLabel,
} from '@iota/core';
import {
    Button,
    ButtonSize,
    ButtonType,
    Address,
    InfoBox,
    InfoBoxType,
    InfoBoxStyle,
} from '@iota/apps-ui-kit';
import { Network } from '@iota/iota-sdk/client';
import { formatAddress, IOTA_TYPE_ARG } from '@iota/iota-sdk/utils';
import { useQuery } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { ArrowBottomLeft, Info, Migration, Send, Vesting } from '@iota/apps-ui-icons';
import { Interstitial, type InterstitialConfig } from '../interstitial';
import { CoinBalance } from './coin-balance';
import { TokenStakingOverview } from './TokenStakingOverview';
import { useNavigate } from 'react-router-dom';
import { MyTokens } from './MyTokens';
import { ReceiveTokensDialog } from './ReceiveTokensDialog';
import { OverviewHint } from './OverviewHint';
import { SupplyIncreaseVestingStakingDialog } from './SupplyIncreaseVestingStakingDialog';
import { MigrationDialog } from './MigrationDialog';

export function TokenDetails() {
    const navigate = useNavigate();
    const [dialogReceiveOpen, setDialogReceiveOpen] = useState(false);
    const [dialogVestingOpen, setDialogVestingOpen] = useState(false);
    const [dialogMigrationOpen, setDialogMigrationOpen] = useState(false);
    const [interstitialDismissed, setInterstitialDismissed] = useState<boolean>(false);
    const activeCoinType = IOTA_TYPE_ARG;
    const activeAccount = useActiveAccount();
    const activeAccountAddress = activeAccount?.address;
    const network = useAppSelector((state) => state.app.network);
    const isMainnet = network === Network.Mainnet;
    const supplyIncreaseVestingEnabled = useFeature<boolean>(Feature.SupplyIncreaseVesting).value;
    const migrationEnabled = useFeature<boolean>(Feature.StardustMigration).value;

    const OBJECT_PER_REQ = 1;

    const { request } = useAppsBackend();
    const { data } = useQuery({
        queryKey: ['apps-backend', 'monitor-network'],
        queryFn: () =>
            request<{ degraded: boolean }>('monitor-network', {
                project: 'WALLET',
            }),
        // Keep cached for 2 minutes:
        staleTime: 2 * 60 * 1000,
        retry: false,
        enabled: isMainnet,
    });
    const explorerHref = useExplorerLink({
        type: ExplorerLinkType.Address,
        address: activeAccountAddress,
    });
    const address = useActiveAddress();
    const {
        data: coinBalances,
        isPending,
        isLoading,
        isFetched,
        isError,
    } = useGetAllBalances(address);
    const coinBalance = coinBalances?.find((balance) => balance.coinType === activeCoinType);

    const { data: delegatedStake } = useGetDelegatedStake({
        address: activeAccountAddress || '',
        staleTime: DELEGATED_STAKES_QUERY_STALE_TIME,
        refetchInterval: DELEGATED_STAKES_QUERY_REFETCH_INTERVAL,
    });

    const { data: supplyIncreaseVestingObjects } = useGetOwnedObjects(
        activeAccountAddress || '',
        {
            StructType: TIMELOCK_IOTA_TYPE,
        },
        OBJECT_PER_REQ,
    );
    const { data: supplyIncreaseVestingObjectsStaked } = useGetOwnedObjects(
        activeAccountAddress || '',
        {
            StructType: TIMELOCK_STAKED_TYPE,
        },
        OBJECT_PER_REQ,
    );

    const { data: basicOutputObjects } = useGetOwnedObjects(
        activeAccountAddress || '',
        { StructType: STARDUST_BASIC_OUTPUT_TYPE },
        OBJECT_PER_REQ,
    );

    const { data: nftOutputObjects } = useGetOwnedObjects(
        activeAccountAddress || '',
        { StructType: STARDUST_NFT_OUTPUT_TYPE },
        OBJECT_PER_REQ,
    );

    const { data: stardustSharedBasicObjects } = useGetStardustSharedBasicObjects(
        activeAccountAddress || '',
        OBJECT_PER_REQ,
    );
    const { data: stardustSharedNftObjects } = useGetStardustSharedNftObjects(
        activeAccountAddress || '',
        OBJECT_PER_REQ,
    );

    let hasSupplyIncreaseVestingObjects = false;
    let needsMigration = false;

    if (supplyIncreaseVestingEnabled) {
        hasSupplyIncreaseVestingObjects =
            haveSupplyIncreaseLabel(supplyIncreaseVestingObjects?.pages || []) ||
            haveSupplyIncreaseLabel(supplyIncreaseVestingObjectsStaked?.pages || []);
    }

    if (migrationEnabled) {
        needsMigration =
            !!basicOutputObjects?.pages?.[0]?.data?.length ||
            !!nftOutputObjects?.pages?.[0]?.data?.length ||
            !!stardustSharedBasicObjects?.length ||
            !!stardustSharedNftObjects?.length;
    }

    const walletInterstitialConfig = useFeature<InterstitialConfig>(
        Feature.WalletInterstitialConfig,
    ).value;

    const tokenBalance = BigInt(coinBalance?.totalBalance ?? 0);

    // Avoid perpetual loading state when fetching and retry keeps failing add isFetched check
    const isFirstTimeLoading = isPending && !isFetched;

    const onSendClick = () => {
        if (!activeAccount?.isLocked) {
            const destination = coinBalance?.coinType
                ? `/send?${new URLSearchParams({ type: coinBalance?.coinType }).toString()}`
                : '/send';

            navigate(destination);
        }
    };

    useEffect(() => {
        const dismissed =
            walletInterstitialConfig?.dismissKey &&
            localStorage.getItem(walletInterstitialConfig.dismissKey);
        setInterstitialDismissed(dismissed === 'true');
    }, [walletInterstitialConfig?.dismissKey]);

    if (
        navigator.userAgent !== 'Playwright' &&
        walletInterstitialConfig?.enabled &&
        !interstitialDismissed
    ) {
        return (
            <Interstitial
                {...walletInterstitialConfig}
                onClose={() => {
                    setInterstitialDismissed(true);
                }}
            />
        );
    }
    const accountHasIota = coinBalances?.some(({ coinType }) => coinType === IOTA_TYPE_ARG);

    if (!activeAccountAddress) {
        return null;
    }
    if (isError) {
        toast.error('Error updating balance');
    }
    return (
        <>
            {isMainnet && data?.degraded && (
                <InfoBox
                    icon={<Info className="h-3 w-3" />}
                    title="App Performance"
                    supportingText="We apologize for the slowdown. Our team is working on a fix and appreciates your patience."
                    type={InfoBoxType.Default}
                    style={InfoBoxStyle.Elevated}
                />
            )}
            <Loading loading={isFirstTimeLoading}>
                <div
                    className="flex h-full flex-1 flex-grow flex-col items-center gap-md"
                    data-testid="coin-page"
                >
                    <div className="flex w-full items-center justify-between gap-lg px-sm py-lg">
                        <div className="flex flex-col gap-xs">
                            <Address
                                isExternal={!!explorerHref}
                                externalLink={explorerHref!}
                                text={formatAddress(activeAccountAddress)}
                                isCopyable
                                copyText={activeAccountAddress}
                                onCopySuccess={() => toast('Address copied')}
                            />
                            <CoinBalance amount={tokenBalance} type={activeCoinType} />
                        </div>
                        <div className="flex gap-xs [&_svg]:h-5 [&_svg]:w-5">
                            <Button
                                onClick={() => setDialogReceiveOpen(true)}
                                type={ButtonType.Secondary}
                                icon={<ArrowBottomLeft />}
                                size={ButtonSize.Small}
                            />
                            <Button
                                onClick={onSendClick}
                                icon={<Send />}
                                size={ButtonSize.Small}
                                disabled={activeAccount?.isLocked || !coinBalances?.length}
                                testId="send-coin-button"
                            />
                        </div>
                    </div>
                    {activeAccount.isLocked ? (
                        <UnlockAccountButton account={activeAccount} />
                    ) : (
                        <div className="flex w-full flex-grow flex-col gap-md">
                            <div
                                className={`flex w-full flex-col items-center gap-xs rounded-2xl ${!accountHasIota ? 'flex-grow justify-between' : ''}`}
                            >
                                <div className="flex w-full flex-col items-center gap-xs">
                                    {accountHasIota || delegatedStake?.length ? (
                                        <TokenStakingOverview
                                            disabled={!tokenBalance}
                                            accountAddress={activeAccountAddress}
                                        />
                                    ) : null}
                                    {hasSupplyIncreaseVestingObjects || needsMigration ? (
                                        <div className="flex w-full flex-row gap-x-xs">
                                            {hasSupplyIncreaseVestingObjects ? (
                                                <OverviewHint
                                                    onClick={() => setDialogVestingOpen(true)}
                                                    title="Vested Staking"
                                                    icon={Vesting}
                                                />
                                            ) : null}
                                            {needsMigration ? (
                                                <OverviewHint
                                                    onClick={() => setDialogMigrationOpen(true)}
                                                    title="Migration"
                                                    icon={Migration}
                                                />
                                            ) : null}
                                        </div>
                                    ) : null}
                                </div>
                                {!accountHasIota ? (
                                    <div className="flex flex-col gap-md">
                                        <div className="flex flex-col flex-nowrap items-center justify-center px-sm text-center">
                                            <span className="text-body-sm text-neutral-40 dark:text-neutral-60">
                                                {isMainnet
                                                    ? 'Start by buying IOTA'
                                                    : 'Need to send transactions on the IOTA network? You’ll need IOTA in your wallet'}
                                            </span>
                                        </div>
                                        {!isMainnet && <FaucetRequestButton />}
                                    </div>
                                ) : null}
                            </div>
                            {coinBalances?.length ? (
                                <MyTokens
                                    coinBalances={coinBalances ?? []}
                                    isLoading={isLoading}
                                    isFetched={isFetched}
                                />
                            ) : null}
                        </div>
                    )}
                </div>
                <ReceiveTokensDialog
                    address={activeAccountAddress}
                    open={dialogReceiveOpen}
                    setOpen={(isOpen) => setDialogReceiveOpen(isOpen)}
                />
                <SupplyIncreaseVestingStakingDialog
                    open={dialogVestingOpen}
                    setOpen={(isOpen) => setDialogVestingOpen(isOpen)}
                />
                <MigrationDialog
                    open={dialogMigrationOpen}
                    setOpen={(isOpen) => setDialogMigrationOpen(isOpen)}
                />
            </Loading>
        </>
    );
}
