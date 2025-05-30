// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import { type Config } from 'tailwindcss';
// Note: exception for the tailwind preset import
import uiKitStaticPreset from '../../apps/ui-kit/src/lib/tailwind/static.presets';

export default {
    presets: [uiKitStaticPreset],
    content: [
        './src/**/*.{js,jsx,ts,tsx}',
        './../ui-kit/src/lib/**/*.{js,jsx,ts,tsx}',
        './../core/src/components/**/*.{js,jsx,ts,tsx}',
    ],
    theme: {
        extend: {
            colors: {
                'gradient-blue-start': '#589AEA',
                'gradient-blue-end': '#4C75A6',
                facebook: '#1877F2',
                twitch: '#6441A5',
                kakao: '#FEE500',
            },
            minHeight: {
                8: '2rem',
                15: '3.75rem',
                'popup-minimum': '600px',
            },
            spacing: {
                7.5: '1.875rem',
                8: '2rem',
                15: '3.75rem',
                'popup-height': '680px',
                'popup-width': '360px',
                'nav-height': '60px',
            },
            boxShadow: {
                'wallet-content': '0px -5px 20px 5px rgba(160, 182, 195, 0.15)',
                button: '0px 1px 2px rgba(16, 24, 40, 0.05)',
                notification: '0px 0px 20px rgba(29, 55, 87, 0.11)',
                'wallet-modal': '0px 0px 44px 0px rgba(0, 0, 0, 0.15)',
                'card-soft': '1px 2px 8px 2px rgba(21, 82, 123, 0.05)',
            },
            borderRadius: {
                20: '1.25rem',
                15: '0.9375rem',
                '2lg': '0.625rem',
                '3lg': '0.75rem',
                '4lg': '1rem',
            },
            gridTemplateColumns: {
                header: '1fr fit-content(200px) 1fr',
            },
            height: {
                header: '4.25rem',
            },
            maxWidth: {
                'popup-width': '360px',
                'token-width': '80px',
            },
            dropShadow: {
                accountModal: [
                    '0px 10px 30px rgba(0, 0, 0, 0.15)',
                    '0px 10px 50px rgba(0, 0, 0, 0.15)',
                ],
            },
            backgroundImage: {
                'twitch-image': 'linear-gradient(165deg, #ECE5FA 5.6%, #C8BAE2 89.58%);',
            },
        },
    },
} satisfies Partial<Config>;
