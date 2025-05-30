// Copyright (c) 2025 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::{fmt, str::FromStr};

use iota_types::base_types::IotaAddress;
use move_core_types::{ident_str, identifier::IdentStr, language_storage::StructTag};
use serde::{Deserialize, Serialize};

use crate::{
    constants::{
        IOTA_NAMES_MAX_DOMAIN_LENGTH, IOTA_NAMES_MAX_LABEL_LENGTH, IOTA_NAMES_MIN_LABEL_LENGTH,
        IOTA_NAMES_SEPARATOR_AT, IOTA_NAMES_SEPARATOR_DOT, IOTA_NAMES_TLD,
    },
    error::IotaNamesError,
};

#[derive(Debug, Serialize, Deserialize, Clone, Eq, Hash, PartialEq)]
pub struct Domain {
    // Labels of the domain name, in reverse order
    labels: Vec<String>,
}

impl FromStr for Domain {
    type Err = IotaNamesError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() > IOTA_NAMES_MAX_DOMAIN_LENGTH {
            return Err(IotaNamesError::DomainLengthExceeded(
                s.len(),
                IOTA_NAMES_MAX_DOMAIN_LENGTH,
            ));
        }

        let formatted_string = convert_from_at_format(s, &IOTA_NAMES_SEPARATOR_DOT)?;

        let labels = formatted_string
            .split(IOTA_NAMES_SEPARATOR_DOT)
            .rev()
            .map(validate_label)
            .collect::<Result<Vec<_>, Self::Err>>()?;

        // A valid domain in our system has at least a TLD and an SLD (len == 2).
        if labels.len() < 2 {
            return Err(IotaNamesError::NotEnoughLabels);
        }

        if labels[0] != IOTA_NAMES_TLD {
            return Err(IotaNamesError::InvalidTld(labels[0].to_string()));
        }

        let labels = labels.into_iter().map(ToOwned::to_owned).collect();

        Ok(Domain { labels })
    }
}

impl fmt::Display for Domain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // We use to_string() to check on-chain state and parse on-chain data
        // so we should always default to DOT format.
        let output = self.format(DomainFormat::Dot);
        f.write_str(&output)?;

        Ok(())
    }
}

impl Domain {
    pub fn type_(package_address: IotaAddress) -> StructTag {
        const IOTA_NAMES_DOMAIN_MODULE: &IdentStr = ident_str!("domain");
        const IOTA_NAMES_DOMAIN_STRUCT: &IdentStr = ident_str!("Domain");

        StructTag {
            address: package_address.into(),
            module: IOTA_NAMES_DOMAIN_MODULE.to_owned(),
            name: IOTA_NAMES_DOMAIN_STRUCT.to_owned(),
            type_params: vec![],
        }
    }

    /// Derive the parent domain for a given domain. Only subdomains have
    /// parents; second-level domains return `None`.
    ///
    /// ```
    /// # use std::str::FromStr;
    /// # use iota_names::domain::Domain;
    /// assert_eq!(
    ///     Domain::from_str("test.example.iota").unwrap().parent(),
    ///     Some(Domain::from_str("example.iota").unwrap())
    /// );
    /// assert_eq!(
    ///     Domain::from_str("sub.test.example.iota").unwrap().parent(),
    ///     Some(Domain::from_str("test.example.iota").unwrap())
    /// );
    /// assert_eq!(Domain::from_str("example.iota").unwrap().parent(), None);
    /// ```
    pub fn parent(&self) -> Option<Self> {
        if self.is_subdomain() {
            Some(Self {
                labels: self
                    .labels
                    .iter()
                    .take(self.num_labels() - 1)
                    .cloned()
                    .collect(),
            })
        } else {
            None
        }
    }

    /// Returns whether this domain is a second-level domain (Ex. `test.iota`)
    pub fn is_sld(&self) -> bool {
        self.num_labels() == 2
    }

    /// Returns whether this domain is a subdomain (Ex. `sub.test.iota`)
    pub fn is_subdomain(&self) -> bool {
        self.num_labels() >= 3
    }

    /// Returns the number of labels including TLD.
    ///
    /// ```
    /// # use std::str::FromStr;
    /// # use iota_names::domain::Domain;
    /// assert_eq!(
    ///     Domain::from_str("test.example.iota").unwrap().num_labels(),
    ///     3
    /// )
    /// ```
    pub fn num_labels(&self) -> usize {
        self.labels.len()
    }

    /// Get the label at the given index
    pub fn label(&self, index: usize) -> Option<&String> {
        self.labels.get(index)
    }

    /// Get all of the labels. NOTE: These are in reverse order starting with
    /// the top-level domain and proceeding to subdomains.
    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    /// Formats a domain into a string based on the available output formats.
    /// The default separator is `.`
    pub fn format(&self, format: DomainFormat) -> String {
        let mut labels = self.labels.clone();
        let sep = &IOTA_NAMES_SEPARATOR_DOT.to_string();
        labels.reverse();

        if format == DomainFormat::Dot {
            // DOT format, all labels joined together with dots, including the TLD.
            labels.join(sep)
        } else {
            // SAFETY: This is a safe operation because we only allow a
            // domain's label vector size to be >= 2 (see `Domain::from_str`)
            let _tld = labels.pop();
            let sld = labels.pop().unwrap();

            // AT format, labels minus SLD joined together with dots, then joined to SLD
            // with @, no TLD.
            format!("{}{IOTA_NAMES_SEPARATOR_AT}{sld}", labels.join(sep))
        }
    }
}

/// Two different view options for a domain.
/// `At` -> `test@example` | `Dot` -> `test.example.iota`
#[derive(Clone, Eq, PartialEq, Debug)]
pub enum DomainFormat {
    At,
    Dot,
}

/// Converts @label ending to label{separator}iota ending.
///
/// E.g. `@example` -> `example.iota` | `test@example` -> `test.example.iota`
fn convert_from_at_format(s: &str, separator: &char) -> Result<String, IotaNamesError> {
    let mut splits = s.split(IOTA_NAMES_SEPARATOR_AT);

    let Some(before) = splits.next() else {
        return Err(IotaNamesError::InvalidSeparator);
    };

    let Some(after) = splits.next() else {
        return Ok(before.to_string());
    };

    if splits.next().is_some() || after.contains(*separator) || after.is_empty() {
        return Err(IotaNamesError::InvalidSeparator);
    }

    let mut parts = vec![];

    if !before.is_empty() {
        parts.push(before);
    }

    parts.push(after);
    parts.push(IOTA_NAMES_TLD);

    Ok(parts.join(&separator.to_string()))
}

/// Checks the validity of a label according to these rules:
/// - length must be in
///   [IOTA_NAMES_MIN_LABEL_LENGTH..IOTA_NAMES_MAX_LABEL_LENGTH]
/// - must contain only '0'..'9', 'a'..'z' and '-'
/// - must not start or end with '-'
pub fn validate_label(label: &str) -> Result<&str, IotaNamesError> {
    let bytes = label.as_bytes();
    let len = bytes.len();

    if !(IOTA_NAMES_MIN_LABEL_LENGTH..=IOTA_NAMES_MAX_LABEL_LENGTH).contains(&len) {
        return Err(IotaNamesError::InvalidLabelLength(
            len,
            IOTA_NAMES_MIN_LABEL_LENGTH,
            IOTA_NAMES_MAX_LABEL_LENGTH,
        ));
    }

    for (i, character) in bytes.iter().enumerate() {
        match character {
            b'a'..=b'z' | b'0'..=b'9' => continue,
            b'-' if i == 0 || i == len - 1 => {
                return Err(IotaNamesError::HyphensAsFirstOrLastLabelChar);
            }
            _ => return Err(IotaNamesError::InvalidLabelChar),
        };
    }

    Ok(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_extraction() {
        let name = Domain::from_str("leaf.node.test.iota")
            .unwrap()
            .parent()
            .unwrap();

        assert_eq!(name.to_string(), "node.test.iota");

        let name = name.parent().unwrap();

        assert_eq!(name.to_string(), "test.iota");

        assert!(name.parent().is_none());
    }

    #[test]
    fn name_service_outputs() {
        assert_eq!("@test".parse::<Domain>().unwrap().to_string(), "test.iota");
        assert_eq!(
            "test.iota".parse::<Domain>().unwrap().to_string(),
            "test.iota"
        );
        assert_eq!(
            "test@sld".parse::<Domain>().unwrap().to_string(),
            "test.sld.iota"
        );
        assert_eq!(
            "test.test@example".parse::<Domain>().unwrap().to_string(),
            "test.test.example.iota"
        );
        assert_eq!(
            "iota@iota".parse::<Domain>().unwrap().to_string(),
            "iota.iota.iota"
        );
        assert_eq!("@iota".parse::<Domain>().unwrap().to_string(), "iota.iota");
        assert_eq!(
            "test.test.iota".parse::<Domain>().unwrap().to_string(),
            "test.test.iota"
        );
        assert_eq!(
            "test.test.test.iota".parse::<Domain>().unwrap().to_string(),
            "test.test.test.iota"
        );
    }

    #[test]
    fn invalid_inputs() {
        assert!(".".parse::<Domain>().is_err());
        assert!("@".parse::<Domain>().is_err());
        assert!("@inner.iota".parse::<Domain>().is_err());
        assert!("test@".parse::<Domain>().is_err());
        assert!("iota".parse::<Domain>().is_err());
        assert!("test.test@example.iota".parse::<Domain>().is_err());
        assert!("test@test@example".parse::<Domain>().is_err());
        assert!("test.atoi".parse::<Domain>().is_err());
    }

    #[test]
    fn outputs() {
        let mut domain = "test.iota".parse::<Domain>().unwrap();
        assert!(domain.format(DomainFormat::Dot) == "test.iota");
        assert!(domain.format(DomainFormat::At) == "@test");

        domain = "test.test.iota".parse::<Domain>().unwrap();
        assert!(domain.format(DomainFormat::Dot) == "test.test.iota");
        assert!(domain.format(DomainFormat::At) == "test@test");

        domain = "test.test.test.iota".parse::<Domain>().unwrap();
        assert!(domain.format(DomainFormat::Dot) == "test.test.test.iota");
        assert!(domain.format(DomainFormat::At) == "test.test@test");

        domain = "test.test.test.test.iota".parse::<Domain>().unwrap();
        assert!(domain.format(DomainFormat::Dot) == "test.test.test.test.iota");
        assert!(domain.format(DomainFormat::At) == "test.test.test@test");
    }
}
