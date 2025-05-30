// Copyright (c) The Diem Core Contributors
// Copyright (c) The Move Contributors
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{anyhow, bail, format_err, Context, Result};
use move_compiler::editions::{Edition, Flavor};
use move_core_types::account_address::{AccountAddress, AccountAddressParseError};
use move_symbol_pool::symbol::Symbol;
use toml::Value as TV;

use super::layout::SourcePackageLayout;
use crate::{package_hooks, source_package::parsed_manifest as PM};

const EMPTY_ADDR_STR: &str = "_";

pub const PACKAGE_NAME: &str = "package";
const BUILD_NAME: &str = "build";
const ADDRESSES_NAME: &str = "addresses";
const DEV_ADDRESSES_NAME: &str = "dev-addresses";
const DEPENDENCY_NAME: &str = "dependencies";
const DEV_DEPENDENCY_NAME: &str = "dev-dependencies";

const EXTERNAL_RESOLVER_PREFIX: &str = "r";

const KNOWN_NAMES: &[&str] = &[
    PACKAGE_NAME,
    BUILD_NAME,
    ADDRESSES_NAME,
    DEV_ADDRESSES_NAME,
    DEPENDENCY_NAME,
    DEV_DEPENDENCY_NAME,
    EXTERNAL_RESOLVER_PREFIX,
];

const REQUIRED_FIELDS: &[&str] = &[PACKAGE_NAME];

pub fn parse_move_manifest_from_file(path: &Path) -> Result<PM::SourceManifest> {
    let toml_path = resolve_move_manifest_path(path);
    let file_contents = std::fs::read_to_string(toml_path)
        .with_context(|| format!("Unable to find package manifest at {:?}", path))?;
    parse_source_manifest(parse_move_manifest_string(file_contents)?)
}

pub fn resolve_move_manifest_path(path: &Path) -> PathBuf {
    if path.is_file() {
        path.into()
    } else {
        path.join(SourcePackageLayout::Manifest.path())
    }
}

pub fn parse_move_manifest_string(manifest_string: String) -> Result<TV> {
    toml::from_str::<TV>(&manifest_string).context("Unable to parse Move package manifest")
}

pub fn parse_source_manifest(tval: TV) -> Result<PM::SourceManifest> {
    match tval {
        TV::Table(mut table) => {
            check_for_required_field_names(&table, REQUIRED_FIELDS)
                .context("Error parsing package manifest")?;
            warn_if_unknown_field_names(&table, KNOWN_NAMES);
            let addresses = table
                .remove(ADDRESSES_NAME)
                .map(parse_addresses)
                .transpose()
                .context("Error parsing '[addresses]' section of manifest")?;
            let dev_address_assignments = table
                .remove(DEV_ADDRESSES_NAME)
                .map(parse_dev_addresses)
                .transpose()
                .context("Error parsing '[dev-addresses]' section of manifest")?;
            let package = table
                .remove(PACKAGE_NAME)
                .map(parse_package_info)
                .transpose()
                .context("Error parsing '[package]' section of manifest")?
                .unwrap();
            let build = table
                .remove(BUILD_NAME)
                .map(parse_build_info)
                .transpose()
                .context("Error parsing '[build]' section of manifest")?;
            let dependencies = table
                .remove(DEPENDENCY_NAME)
                .map(parse_dependencies)
                .transpose()
                .context("Error parsing '[dependencies]' section of manifest")?
                .unwrap_or_default();
            let dev_dependencies = table
                .remove(DEV_DEPENDENCY_NAME)
                .map(parse_dependencies)
                .transpose()
                .context("Error parsing '[dev-dependencies]' section of manifest")?
                .unwrap_or_default();
            Ok(PM::SourceManifest {
                package,
                addresses,
                dev_address_assignments,
                build,
                dependencies,
                dev_dependencies,
            })
        }
        x => {
            bail!(
                "Malformed package manifest {}. Expected a table at top level, but encountered a {}",
                x,
                x.type_str()
            )
        }
    }
}

pub const EDITION_NAME: &str = "edition";

pub fn parse_package_info(tval: TV) -> Result<PM::PackageInfo> {
    match tval {
        TV::Table(mut table) => {
            check_for_required_field_names(&table, &["name"])?;
            let hook_names = package_hooks::custom_package_info_fields();
            let known_names = ["name", "authors", "license", EDITION_NAME, "flavor"]
                .into_iter()
                .chain(hook_names.iter().map(|s| s.as_str()))
                .collect::<Vec<_>>();
            warn_if_unknown_field_names(&table, known_names.as_slice());
            let name = table
                .remove("name")
                .ok_or_else(|| format_err!("'name' is a required field but was not found",))?;
            let name = name
                .as_str()
                .ok_or_else(|| format_err!("Package name must be a string"))?;
            let name = PM::PackageName::from(name);
            let license = table.remove("license").map(|x| Symbol::from(x.to_string()));
            let authors = match table.remove("authors") {
                None => Vec::new(),
                Some(arr) => {
                    let unparsed_vec = arr
                        .as_array()
                        .ok_or_else(|| format_err!("Invalid author(s) list"))?;
                    unparsed_vec
                        .iter()
                        .map(|tval| {
                            tval.as_str()
                                .map(|x| Symbol::from(x.to_string()))
                                .ok_or_else(|| {
                                    format_err!(
                                        "Invalid author '{}' of type {} found. Expected a string.",
                                        tval.to_string(),
                                        tval.type_str()
                                    )
                                })
                        })
                        .collect::<Result<_>>()?
                }
            };
            let edition = table
                .remove("edition")
                .map(|v| {
                    let s = v
                        .as_str()
                        .ok_or_else(|| format_err!("'edition' must be a string"))?;
                    Edition::from_str(s).map_err(|err| format_err!("Invalid 'edition'. {err}"))
                })
                .transpose()?;
            let flavor = table
                .remove("flavor")
                .map(|v| {
                    let s = v
                        .as_str()
                        .ok_or_else(|| format_err!("'flavor' must be a string"))?;
                    Flavor::from_str(s).map_err(|err| format_err!("Invalid 'flavor'. {err}"))
                })
                .transpose()?;
            // Turn the remaining entries into custom properties. For those which are not
            // supported (also in the presence of hooks) we have warned above.
            let mut custom_properties: BTreeMap<Symbol, String> = Default::default();
            for (name, val) in table {
                let val_str = val
                    .as_str()
                    .ok_or_else(|| format_err!("Field `{}` value must be a string", name))?;
                custom_properties.insert(Symbol::from(name), val_str.to_owned());
            }

            Ok(PM::PackageInfo {
                name,
                authors,
                license,
                custom_properties,
                edition,
                flavor,
            })
        }
        x => bail!(
            "Malformed section in manifest {}. Expected a table, but encountered a {}",
            x,
            x.type_str()
        ),
    }
}

pub fn parse_dependencies(tval: TV) -> Result<PM::Dependencies> {
    match tval {
        TV::Table(table) => {
            let mut deps = BTreeMap::new();
            for (dep_name, dep) in table.into_iter() {
                let dep_name_ident = PM::PackageName::from(dep_name.clone());
                let dep = parse_dependency(dep)?;
                deps.insert(dep_name_ident, dep);
            }
            Ok(deps)
        }
        x => bail!(
            "Malformed section in manifest {}. Expected a table, but encountered a {}",
            x,
            x.type_str()
        ),
    }
}

pub fn parse_build_info(tval: TV) -> Result<PM::BuildInfo> {
    match tval {
        TV::Table(mut table) => {
            warn_if_unknown_field_names(&table, &["language_version", "arch"]);
            Ok(PM::BuildInfo {
                language_version: table
                    .remove("language_version")
                    .map(parse_version)
                    .transpose()?,
            })
        }
        x => bail!(
            "Malformed section in manifest {}. Expected a table, but encountered a {}",
            x,
            x.type_str()
        ),
    }
}

pub fn parse_addresses(tval: TV) -> Result<PM::AddressDeclarations> {
    match tval {
        TV::Table(table) => {
            let mut addresses = BTreeMap::new();
            for (addr_name, entry) in table.into_iter() {
                let ident = PM::NamedAddress::from(addr_name);
                match entry.as_str() {
                    Some(entry_str) => {
                        if entry_str == EMPTY_ADDR_STR {
                            if addresses.insert(ident, None).is_some() {
                                bail!("Duplicate address name '{}' found.", ident);
                            }
                        } else if addresses
                            .insert(
                                ident,
                                Some(parse_address_literal(entry_str).context(format!(
                                    "Invalid address '{}' encountered.",
                                    entry_str
                                ))?),
                            )
                            .is_some()
                        {
                            bail!("Duplicate address name '{}' found.", ident);
                        }
                    }
                    None => bail!(
                        "Invalid address name {} encountered. Expected a string but found a {}",
                        entry,
                        entry.type_str()
                    ),
                }
            }
            Ok(addresses)
        }
        x => bail!(
            "Malformed section in manifest {}. Expected a table, but encountered a {}",
            x,
            x.type_str()
        ),
    }
}

pub fn parse_dev_addresses(tval: TV) -> Result<PM::DevAddressDeclarations> {
    match tval {
        TV::Table(table) => {
            let mut addresses = BTreeMap::new();
            for (addr_name, entry) in table.into_iter() {
                let ident = PM::NamedAddress::from(addr_name);
                match entry.as_str() {
                    Some(entry_str) => {
                        if entry_str == EMPTY_ADDR_STR {
                            bail!(
                                "Found uninstantiated named address '{}'. All addresses in the '{}' field must be instantiated.",
                                ident,
                                DEV_ADDRESSES_NAME
                            );
                        } else if addresses
                            .insert(
                                ident,
                                parse_address_literal(entry_str).context(format!(
                                    "Invalid address '{}' encountered.",
                                    entry_str
                                ))?,
                            )
                            .is_some()
                        {
                            bail!("Duplicate address name '{}' found.", ident);
                        }
                    }
                    None => bail!(
                        "Invalid address name {} encountered. Expected a string but found a {}",
                        entry,
                        entry.type_str()
                    ),
                }
            }
            Ok(addresses)
        }
        x => bail!(
            "Malformed section in manifest {}. Expected a table, but encountered a {}",
            x,
            x.type_str()
        ),
    }
}

// Safely parses address for both the 0x and non prefixed hex format.
fn parse_address_literal(address_str: &str) -> Result<AccountAddress, AccountAddressParseError> {
    if !address_str.starts_with("0x") {
        return AccountAddress::from_hex(address_str);
    }
    AccountAddress::from_hex_literal(address_str)
}

fn parse_external_resolver_name(resolver_val: &TV) -> Result<Option<Symbol>> {
    let Some(table) = resolver_val.as_table() else {
        bail!("Malformed dependency {}", resolver_val);
    };

    if table.len() != 1 {
        bail!("Malformed external resolver declaration for dependency {EXTERNAL_RESOLVER_PREFIX}.{resolver_val}",);
    }

    let key = table
        .keys()
        .next()
        .expect("Exactly one key by check above")
        .as_str();

    let key_value = table.get(key).ok_or_else(|| {
        format_err!("Malformed external resolver declaration for dependency {EXTERNAL_RESOLVER_PREFIX}.{resolver_val}",)
    })?;

    if !key_value.is_str() {
        bail!("Malformed external resolver declaration for dependency {EXTERNAL_RESOLVER_PREFIX}.{resolver_val}",);
    }

    Ok(Some(Symbol::from(key)))
}

pub fn parse_dependency(mut tval: TV) -> Result<PM::Dependency> {
    let Some(table) = tval.as_table_mut() else {
        bail!("Malformed dependency {}", tval);
    };

    if let Some(external_resolver_binary_name) = table
        .get(EXTERNAL_RESOLVER_PREFIX)
        .and_then(|e| parse_external_resolver_name(e).transpose())
    {
        return Ok(PM::Dependency::External(external_resolver_binary_name?));
    }

    let subst = table
        .remove("addr_subst")
        .map(parse_substitution)
        .transpose()?;
    let digest = table.remove("digest").map(parse_digest).transpose()?;
    let dep_override = table
        .remove("override")
        .map(parse_dep_override)
        .transpose()?
        .map_or(false, |o| o);

    let kind = match (
        table.remove("local"),
        table.remove("subdir"),
        table.remove("git"),
        table.remove("id"),
    ) {
        (Some(local), subdir, None, None) => {
            if subdir.is_some() {
                bail!("'subdir' not supported for local dependencies");
            }

            let Some(local) = local.as_str().map(PathBuf::from) else {
                bail!("Local source path not a string")
            };

            PM::DependencyKind::Local(
                // with allow_cwd_parent set to true, it never fails
                PM::normalize_path(local, true /* allow_cwd_parent */).unwrap(),
            )
        }

        (None, subdir, Some(git_url), None) => {
            let Some(git_rev) = table.remove("rev") else {
                bail!("Git revision not supplied for dependency")
            };

            let Some(git_rev) = git_rev.as_str().map(Symbol::from) else {
                bail!("Git revision not a string")
            };

            let Some(git_url) = git_url.as_str().map(Symbol::from) else {
                bail!("Git URL not a string")
            };

            let subdir = match subdir {
                None => PathBuf::new(),
                Some(path) => path
                    .as_str()
                    .map(PathBuf::from)
                    .ok_or_else(|| anyhow!("'subdir' not a string"))?,
            };

            PM::DependencyKind::Git(PM::GitInfo {
                git_url,
                git_rev,
                subdir,
            })
        }

        (None, None, None, Some(id)) => {
            let Some(id) = id.as_str().map(Symbol::from) else {
                bail!("ID not a string")
            };

            PM::DependencyKind::OnChain(PM::OnChainInfo { id })
        }

        _ => {
            let keys = ["'local'", "'git'", "'r.<external_resolver_binary_name>'"];
            bail!(
                "must provide exactly one of {} for dependency.",
                keys.join(" or ")
            )
        }
    };

    // Any fields that are left are unknown
    warn_if_unknown_field_names(table, &[]);

    Ok(PM::Dependency::Internal(PM::InternalDependency {
        kind,
        subst,
        digest,
        dep_override,
    }))
}

pub fn parse_substitution(tval: TV) -> Result<PM::Substitution> {
    match tval {
        TV::Table(table) => {
            let mut subst = BTreeMap::new();
            for (addr_name, tval) in table.into_iter() {
                let addr_ident = PM::NamedAddress::from(addr_name.as_str());
                match tval {
                    TV::String(addr_or_name) => {
                        if let Ok(addr) = AccountAddress::from_hex_literal(&addr_or_name) {
                            subst.insert(addr_ident, PM::SubstOrRename::Assign(addr));
                        } else {
                            let rename_from = PM::NamedAddress::from(addr_or_name.as_str());
                            subst.insert(addr_ident, PM::SubstOrRename::RenameFrom(rename_from));
                        }
                    }
                    x => bail!(
                        "Malformed dependency substitution {}. Expected a string, but encountered a {}",
                        x,
                        x.type_str()
                    ),
                }
            }
            Ok(subst)
        }
        x => bail!(
            "Malformed dependency substitution {}. Expected a table, but encountered a {}",
            x,
            x.type_str()
        ),
    }
}

fn parse_version(tval: TV) -> Result<PM::Version> {
    let version_str = tval.as_str().unwrap();
    let version_parts = version_str.split('.').collect::<Vec<_>>();
    if version_parts.len() != 3 {
        bail!(
            "Version is malformed. Versions must be of the form <u64>.<u64>.<u64>, but found '{}'",
            version_str
        );
    }

    Ok((
        version_parts[0]
            .parse::<u64>()
            .context("Invalid major version")?,
        version_parts[1]
            .parse::<u64>()
            .context("Invalid minor version")?,
        version_parts[2]
            .parse::<u64>()
            .context("Invalid bugfix version")?,
    ))
}

fn parse_digest(tval: TV) -> Result<PM::PackageDigest> {
    let digest_str = tval
        .as_str()
        .ok_or_else(|| format_err!("Invalid package digest"))?;
    Ok(PM::PackageDigest::from(digest_str))
}

fn parse_dep_override(tval: TV) -> Result<PM::DepOverride> {
    if !tval.is_bool() {
        bail!("Invalid dependency override value");
    }
    Ok(tval.as_bool().unwrap())
}

// Check that only recognized names are provided at the top-level.
fn warn_if_unknown_field_names(table: &toml::map::Map<String, TV>, known_names: &[&str]) {
    let mut unknown_names = BTreeSet::new();
    for key in table.keys() {
        if !known_names.contains(&key.as_str()) {
            unknown_names.insert(key.to_string());
        }
    }

    if !unknown_names.is_empty() {
        eprintln!(
            "Warning: unknown field name{} found. Expected one of [{}], but found {}",
            if unknown_names.len() > 1 { "s" } else { "" },
            known_names.join(", "),
            unknown_names
                .into_iter()
                .map(|x| format!("'{}'", x))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}

fn check_for_required_field_names(
    table: &toml::map::Map<String, TV>,
    required_fields: &[&str],
) -> Result<()> {
    let mut missing_fields = BTreeSet::new();

    for field_name in required_fields {
        if !table.contains_key(*field_name) {
            missing_fields.insert(field_name.to_string());
        }
    }

    if !missing_fields.is_empty() {
        bail!(
            "Required field name{} {} not found",
            if missing_fields.len() > 1 { "s" } else { "" },
            missing_fields
                .into_iter()
                .map(|x| format!("'{}'", x))
                .collect::<Vec<_>>()
                .join(", "),
        )
    }

    Ok(())
}
