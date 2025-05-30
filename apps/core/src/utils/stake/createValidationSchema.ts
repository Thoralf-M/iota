// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import { CoinFormat, formatBalance, MIN_NUMBER_IOTA_TO_STAKE } from '../../';
import BigNumber from 'bignumber.js';
import { mixed, object } from 'yup';

export function createValidationSchema(
    coinBalance: bigint,
    coinSymbol: string,
    decimals: number,
    minimumStake: bigint,
) {
    return object({
        // NOTE: This is an intentional subset of the token validation:
        amount: mixed<BigNumber>()
            .transform((_, original) => {
                return new BigNumber(original);
            })
            .test('required', `\${path} is a required field`, (value) => {
                return !!value;
            })
            .test('valid', 'The value provided is not valid.', (value) => {
                if (!value || value.isNaN() || !value.isFinite()) {
                    return false;
                }
                return true;
            })
            .test(
                'min',
                `\${path} must be greater than ${MIN_NUMBER_IOTA_TO_STAKE} ${coinSymbol}`,
                (amount) =>
                    amount ? amount.shiftedBy(decimals).gte(minimumStake.toString()) : false,
            )
            .test('max', (amount, ctx) => {
                const gasBudget = ctx.parent.gasBudget || 0n;
                const availableBalance = coinBalance - gasBudget;
                if (availableBalance < 0) {
                    return ctx.createError({
                        message: 'Insufficient funds',
                    });
                }

                const canStake = availableBalance >= minimumStake;
                if (!canStake)
                    return ctx.createError({
                        message: `Insufficient funds to stake a minimum of ${MIN_NUMBER_IOTA_TO_STAKE} ${coinSymbol}`,
                    });

                const enoughBalance = amount
                    ? amount.shiftedBy(decimals).lte(availableBalance.toString())
                    : false;
                if (enoughBalance) {
                    return true;
                }
                return ctx.createError({
                    message: `\${path} must be less than ${formatBalance(
                        availableBalance,
                        decimals,
                        CoinFormat.FULL,
                    )} ${coinSymbol}`,
                });
            })
            .test(
                'max-decimals',
                `The value exceeds the maximum decimals (${decimals}).`,
                (amount) => {
                    return amount ? amount.shiftedBy(decimals).isInteger() : false;
                },
            )
            .label('Amount'),
    });
}
