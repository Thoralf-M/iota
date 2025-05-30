// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import { useIotaClientQuery } from '@iota/dapp-kit';
import { FieldItem } from './FieldItem';
import type { DynamicFieldName } from '@iota/iota-sdk/client';
import { InfoBox, InfoBoxStyle, InfoBoxType, LoadingIndicator } from '@iota/apps-ui-kit';
import { Warning } from '@iota/apps-ui-icons';

interface UnderlyingObjectCardProps {
    parentId: string;
    name: DynamicFieldName;
    dynamicFieldType: 'DynamicField' | 'DynamicObject';
}

export function UnderlyingObjectCard({
    parentId,
    name,
    dynamicFieldType,
}: UnderlyingObjectCardProps): JSX.Element | null {
    const { data, isPending, isError, isFetched } = useIotaClientQuery('getDynamicFieldObject', {
        parentId,
        name,
    });
    const objectType =
        data?.data?.type ??
        (data?.data?.content?.dataType === 'package' ? 'package' : data?.data?.content?.type) ??
        null;
    // Get the packageId, moduleName, functionName from the objectType
    const [packageId, moduleName, functionName] = objectType?.split('<')[0]?.split('::') || [];

    // Get the normalized struct for the object
    const {
        data: normalizedStruct,
        isFetched: normalizedStructFetched,
        isPending: loadingNormalizedStruct,
    } = useIotaClientQuery('getNormalizedMoveStruct', {
        package: packageId,
        module: moduleName,
        struct: functionName,
    });

    const isDataLoading = isPending || loadingNormalizedStruct;

    // Check for error first before showing the loading spinner to avoid infinite loading if GetDynamicFieldObject fails
    if (
        !isDataLoading &&
        (isError ||
            (data && data.error) ||
            (isFetched && !data) ||
            (!normalizedStruct && normalizedStructFetched))
    ) {
        return (
            <InfoBox
                title="Error loading data"
                supportingText={`Failed to get field data for ${parentId}`}
                icon={<Warning />}
                type={InfoBoxType.Error}
                style={InfoBoxStyle.Elevated}
            />
        );
    }

    if (isDataLoading) {
        return (
            <div className="mt-3 flex w-full justify-center pt-3">
                <LoadingIndicator text="Loading data" />
            </div>
        );
    }

    const fieldsData =
        data?.data?.content?.dataType === 'moveObject' ? data.data?.content.fields : null;
    // Return null if there are no fields
    if (!fieldsData || !normalizedStruct?.fields || !objectType) {
        return null;
    }
    // For dynamicObject type show the entire object
    const fieldData =
        dynamicFieldType === 'DynamicObject'
            ? fieldsData
            : (fieldsData as { value?: unknown })?.value;

    const dynamicFieldsData =
        // show name if it is a struct
        typeof name.value === 'object' ? { name, value: fieldData } : fieldData;

    return (
        <FieldItem
            value={dynamicFieldsData as string}
            objectType={objectType}
            // add the struct type to the value
            type={normalizedStruct?.fields.find((field) => field.name === 'value')?.type || ''}
        />
    );
}
