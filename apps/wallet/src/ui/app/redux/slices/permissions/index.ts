// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import type { Permission } from '_messages/payloads/permissions';
import type { RootState } from '_src/ui/app/redux/rootReducer';
import type { AppThunkConfig } from '_src/ui/app/redux/store/thunkExtras';
import { createAsyncThunk, createEntityAdapter, createSlice } from '@reduxjs/toolkit';
import type { PayloadAction } from '@reduxjs/toolkit';

const permissionsAdapter = createEntityAdapter<Permission>({
    sortComparer: (a, b) => {
        const aDate = new Date(a.createdDate);
        const bDate = new Date(b.createdDate);
        return aDate.getTime() - bDate.getTime();
    },
});

export const respondToPermissionRequest = createAsyncThunk<
    {
        id: string;
        accounts: string[];
        allowed: boolean;
        responseDate: string;
    },
    { id: string; accounts: string[]; allowed: boolean },
    AppThunkConfig
>('respond-to-permission-request', ({ id, accounts, allowed }, { extra: { background } }) => {
    const responseDate = new Date().toISOString();
    background.sendPermissionResponse(id, accounts, allowed, responseDate);
    return { id, accounts, allowed, responseDate };
});

const slice = createSlice({
    name: 'permissions',
    initialState: permissionsAdapter.getInitialState({ initialized: false }),
    reducers: {
        setPermissions: (state, { payload }: PayloadAction<Permission[]>) => {
            permissionsAdapter.setAll(state, payload);
            state.initialized = true;
        },
    },
    extraReducers: (build) => {
        build.addCase(respondToPermissionRequest.fulfilled, (state, { payload }) => {
            const { id, accounts, allowed, responseDate } = payload;
            permissionsAdapter.updateOne(state, {
                id,
                changes: {
                    accounts,
                    allowed,
                    responseDate,
                },
            });
        });
    },
});

export default slice.reducer;

export const { setPermissions } = slice.actions;

export const permissionsSelectors = permissionsAdapter.getSelectors(
    (state: RootState) => state.permissions,
);
