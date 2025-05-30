// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import { Link } from '~/components/ui';

type LinkGroupProps = {
    title: string;
} & ({ text: string | null } | { links: { text: string; to: string }[] });

export function LinkGroup(props: LinkGroupProps): JSX.Element | null {
    const { title } = props;
    const isLinks = 'links' in props;
    const isText = 'text' in props;
    if ((isLinks && !props.links.length) || (isText && !props.text)) {
        return null;
    }
    return (
        <div className="space-y-3">
            <div className="font-semibold text-neutral-40">{title}</div>
            {isLinks
                ? props.links.map(({ text, to }) => (
                      <div key={to}>
                          <Link to={to} variant="mono">
                              {text}
                          </Link>
                      </div>
                  ))
                : null}
            {isText ? (
                <div className="text-pBodySmall font-medium text-neutral-40">{props.text}</div>
            ) : null}
        </div>
    );
}
