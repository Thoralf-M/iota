// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import { ErrorBoundary, MenuContent, Navigation, WalletSettingsButton } from '_components';
import cn from 'clsx';
import { createContext, type ReactNode, useState } from 'react';
import { useAppSelector, useActiveAccount } from '_hooks';
import { AppType } from '../../redux/slices/app/appType';
import { Header } from '../header/Header';
import { Toaster } from '../toaster';
import { IotaLogoMark, Ledger } from '@iota/apps-ui-icons';
import { Link } from 'react-router-dom';
import { isLedgerAccountSerializedUI } from '_src/background/accounts/ledgerAccount';
import { type SerializedUIAccount } from '_src/background/accounts/account';
import { formatAddress } from '@iota/iota-sdk/utils';
import { Badge, BadgeType } from '@iota/apps-ui-kit';
import { isLegacyAccount } from '_src/background/accounts/isLegacyAccount';

export const PageMainLayoutContext = createContext<HTMLDivElement | null>(null);

export interface PageMainLayoutProps {
    children: ReactNode | ReactNode[];
    bottomNavEnabled?: boolean;
    topNavMenuEnabled?: boolean;
    dappStatusEnabled?: boolean;
}

export function PageMainLayout({
    children,
    bottomNavEnabled = false,
    topNavMenuEnabled = false,
}: PageMainLayoutProps) {
    const appType = useAppSelector((state) => state.app.appType);
    const activeAccount = useActiveAccount();
    const isFullScreen = appType === AppType.Fullscreen;
    const [titlePortalContainer, setTitlePortalContainer] = useState<HTMLDivElement | null>(null);
    const isLedgerAccount = activeAccount && isLedgerAccountSerializedUI(activeAccount);
    const isHomePage = window.location.hash === '#/tokens';

    return (
        <div
            className={cn(
                'flex max-h-full w-full flex-1 flex-col flex-nowrap items-stretch justify-center overflow-hidden',
                isFullScreen ? 'rounded-xl' : '',
            )}
        >
            {isHomePage ? (
                <Header
                    leftContent={
                        <LeftContent
                            account={activeAccount}
                            isLedgerAccount={isLedgerAccount}
                            isLocked={activeAccount?.isLocked}
                            isLegacyAccount={isLegacyAccount(activeAccount)}
                        />
                    }
                    middleContent={<div ref={setTitlePortalContainer} />}
                    rightContent={topNavMenuEnabled ? <WalletSettingsButton /> : undefined}
                />
            ) : null}
            <div className="relative flex flex-grow flex-col flex-nowrap overflow-hidden">
                <div className="flex flex-grow flex-col flex-nowrap overflow-y-auto overflow-x-hidden bg-neutral-100 dark:bg-neutral-6">
                    <main
                        className={cn('flex w-full flex-grow flex-col', {
                            'p-5': bottomNavEnabled && isHomePage,
                            'h-full': !isHomePage,
                        })}
                    >
                        <PageMainLayoutContext.Provider value={titlePortalContainer}>
                            <ErrorBoundary>{children}</ErrorBoundary>
                        </PageMainLayoutContext.Provider>
                    </main>
                    <Toaster bottomNavEnabled={bottomNavEnabled} />
                </div>
                {topNavMenuEnabled ? <MenuContent /> : null}
            </div>
            {bottomNavEnabled ? <Navigation /> : null}
        </div>
    );
}

function LeftContent({
    account,
    isLedgerAccount,
    isLocked,
    isLegacyAccount,
}: {
    account: SerializedUIAccount | null;
    isLedgerAccount: boolean | null;
    isLocked?: boolean;
    isLegacyAccount?: boolean;
}) {
    const accountName = account?.nickname ?? formatAddress(account?.address || '');
    const backgroundColor = isLocked ? 'bg-neutral-90' : 'bg-primary-30';
    return (
        <Link
            to="/accounts/manage"
            className="flex flex-row items-center gap-sm p-xs text-pink-200 no-underline"
            data-testid="accounts-manage"
        >
            <div
                className={cn(
                    'flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary-30 [&_svg]:h-5 [&_svg]:w-5 [&_svg]:text-white',
                    backgroundColor,
                )}
            >
                {isLedgerAccount ? <Ledger /> : <IotaLogoMark />}
            </div>
            <span className="line-clamp-1 break-all text-title-sm text-neutral-10 dark:text-neutral-92">
                {accountName}
            </span>
            {isLegacyAccount && <Badge type={BadgeType.Neutral} label="Legacy" />}
        </Link>
    );
}
