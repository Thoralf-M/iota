// Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import {
    useFormatCoin,
    useBalance,
    CoinFormat,
    useCoinMetadata,
    safeParseAmount,
    toast,
    useNewStakeTransaction,
    parseAmount,
    getGasBudgetErrorMessage,
} from '@iota/core';
import { IOTA_DECIMALS, IOTA_TYPE_ARG } from '@iota/iota-sdk/utils';
import { useFormikContext } from 'formik';
import { useSignAndExecuteTransaction } from '@iota/dapp-kit';
import { EnterAmountDialogLayout } from './EnterAmountDialogLayout';
import { ampli } from '@/lib/utils/analytics';
import { ButtonPill, InfoBox, InfoBoxStyle, InfoBoxType } from '@iota/apps-ui-kit';
import { useEffect, useMemo } from 'react';
import { Exclamation } from '@iota/apps-ui-icons';

export interface FormValues {
    amount: string;
}

interface EnterAmountViewProps {
    selectedValidator: string;
    onBack: () => void;
    showActiveStatus?: boolean;
    handleClose: () => void;
    amountWithoutDecimals: bigint;
    senderAddress: string;
    onSuccess: (digest: string) => void;
}

export function EnterAmountView({
    selectedValidator,
    onBack,
    handleClose,
    amountWithoutDecimals,
    senderAddress,
    onSuccess,
}: EnterAmountViewProps): JSX.Element {
    const { mutateAsync: signAndExecuteTransaction } = useSignAndExecuteTransaction();
    const { values, resetForm, setFieldValue } = useFormikContext<FormValues>();

    const coinType = IOTA_TYPE_ARG;
    const { data: metadata } = useCoinMetadata(coinType);
    const decimals = metadata?.decimals ?? 0;

    const { data: iotaBalance } = useBalance(senderAddress);
    const coinBalance = BigInt(iotaBalance?.totalBalance || 0);

    const {
        data: newStakeData,
        isLoading: isTransactionLoading,
        isError,
        error: stakeTransactionError,
    } = useNewStakeTransaction(selectedValidator, amountWithoutDecimals, senderAddress);

    const gasSummary = newStakeData?.gasSummary;

    const { data: maxAmountTransactionData } = useNewStakeTransaction(
        selectedValidator,
        coinBalance,
        senderAddress,
    );
    const maxAmountTxGasBudget = BigInt(maxAmountTransactionData?.gasSummary?.budget ?? 0n);

    useEffect(() => {
        setFieldValue('gasBudget', maxAmountTxGasBudget);
    }, [maxAmountTxGasBudget, setFieldValue]);

    // for user we show available amount as available_balance - gas_budget
    const availableBalance = coinBalance - maxAmountTxGasBudget;
    const [availableBalanceFormatted, availableBalanceFormattedSymbol] = useFormatCoin({
        balance: availableBalance,
        format: CoinFormat.FULL,
    });

    const amount = safeParseAmount(coinType === IOTA_TYPE_ARG ? values.amount : '0', decimals);

    const caption = maxAmountTxGasBudget
        ? `${availableBalanceFormatted} ${availableBalanceFormattedSymbol} Available`
        : '--';

    const gasUnstakeBuffer = maxAmountTxGasBudget * BigInt(2);
    const maxSafeAmount = availableBalance - gasUnstakeBuffer;
    const [maxSafeAmountFormatted, maxSafeAmountSymbol] = useFormatCoin({
        balance: maxSafeAmount,
        format: CoinFormat.FULL,
    });
    const isUnsafeAmount = amount && amount > maxSafeAmount && amount <= availableBalance;

    function setMaxAmount() {
        setFieldValue('amount', availableBalanceFormatted, true);
    }

    function setRecommendedAmount() {
        setFieldValue('amount', maxSafeAmountFormatted, true);
    }

    function handleStake(): void {
        if (!newStakeData?.transaction) {
            toast.error('Stake transaction was not created');
            return;
        }
        signAndExecuteTransaction(
            {
                transaction: newStakeData?.transaction,
            },
            {
                onSuccess: (tx) => {
                    onSuccess(tx.digest);
                    toast.success('Stake transaction has been sent');
                    ampli.stakedIota({
                        stakedAmount: Number(parseAmount(values.amount, IOTA_DECIMALS)),
                    });
                    resetForm();
                },
                onError: () => {
                    toast.error('Stake transaction was not sent');
                },
            },
        );
    }

    const errorMessage = useMemo(() => {
        if (isError) {
            return getGasBudgetErrorMessage(stakeTransactionError);
        } else {
            return undefined;
        }
    }, [stakeTransactionError, isError]);

    return (
        <EnterAmountDialogLayout
            selectedValidator={selectedValidator}
            totalGas={gasSummary?.totalGas}
            senderAddress={senderAddress}
            caption={caption}
            renderInfo={
                isUnsafeAmount ? (
                    <InfoBox
                        type={InfoBoxType.Warning}
                        supportingText={
                            <>
                                Staking your full balance may leave you without enough funds to
                                cover gas fees for future actions like unstaking. To avoid this, we
                                recommend staking up to {maxSafeAmountFormatted}&nbsp;
                                {maxSafeAmountSymbol}.
                                <div>
                                    <span
                                        onClick={setRecommendedAmount}
                                        className="cursor-pointer underline hover:opacity-80"
                                    >
                                        Set recommended amount
                                    </span>
                                </div>
                            </>
                        }
                        style={InfoBoxStyle.Elevated}
                        icon={<Exclamation />}
                    />
                ) : undefined
            }
            isLoading={isTransactionLoading}
            onBack={onBack}
            handleClose={handleClose}
            handleStake={handleStake}
            renderInputAction={<ButtonPill onClick={setMaxAmount}>Max</ButtonPill>}
            errorMessage={errorMessage}
        />
    );
}
