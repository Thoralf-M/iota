---
description: How to run a node. Requirements, configuration parameters, dashboard configuration, and tests.
image: /img/logo/WASP_logo_dark.png
tags:
  - how-to
  - isc
---

# Running a Node

As Wasp is an INX plugin, you must run the wasp node alongside your _hornet node_. You can use the simple docker-compose setup to do so.

## Recommended Hardware Requirements

We recommend that you run the docker image on a server with:

- **CPU**: 8 core.
- **RAM**: 16 GB.
- **Disk space**: ~ 250 GB SSD, depending on your pruning configuration.

## Set Up

Clone and follow the instructions on the [node-docker-setup repo](https://github.com/iotaledger/node-docker-setup).

:::note
This is aimed at production-ready deployment. If you're looking to spawn a local node for testing/development, please see the [local-setup](https://github.com/iotaledger/wasp/tree/develop/tools/local-setup)
:::
