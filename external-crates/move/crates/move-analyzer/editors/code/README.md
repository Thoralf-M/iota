# Move

Provides language support for the Move programming language. For information about Move visit the
language [documentation](https://docs.iota.org/developer/iota-101/move-overview/). It also provides early-stage
support for trace-debugging Move unit tests using a familiar VSCode debugging interface (e.g., stepping
through the code, tracking local variable names, setting line breakpoints).

# How to Install

1. Open a new window in any Visual Studio Code application version 1.61.0 or greater.
2. Open the command palette (`⇧` + `⌘` + `P` on macOS, or use the menu item _View > Command Palette..._) and
   type **Extensions: Install Extensions**. This will open a panel named _Extensions_ in the
   sidebar of your Visual Studio Code window.
3. In the search bar labeled _Search Extensions in Marketplace_, type **IOTA Foundation**. The Move extension
   should appear as one of the option in the list below the search bar. Click **Install**.
4. Open any file that ends in `.move`.

Installation of the extension will also install a platform-specific pre-built move-analyzer binary in
the default directory (see [here](#what-if-i-want-to-use-a-move-analyzer-binary-in-a-different-location)
for information on the location of this directory), overwriting the existing binary if it already exists.
The move-analyzer binary is responsible for the advanced features of this VSCode extension (e.g., go to
definition, type on hover). Please see [Troubleshooting](#troubleshooting) for situations when
the pre-built move-analyzer binary is not available for your platform or if you want to use move-analyzer
binary stored in a different location.

If you want to build, test, and trace Move code using the extension, you must install the `iota` binary on
your machine - see [here](https://docs.iota.org/developer/getting-started/install-iota) for
instructions. The extension assumes that the `iota` binary is in your system path, but you can set
its custom location location using VSCode's settings (`⌘` + `,` on macOS, or use the menu item _Code >
Preferences > Settings_). Search for the `move.iota.path` user setting, set it to the new location of
the `iota` binary, and restart VSCode.

In order to trace-debug Move code execution, the `iota` binary must be built with the `tracing` feature flag.
If your version of the `iota` binary was not built with this feature flag, an attempt to trace test
execution will fail. In this case you may have to build the `iota` binary from source following these
[instructions](https://docs.iota.org/developer/getting-started/install-iota#install-iota-binaries-from-source).

# Troubleshooting

## What if the pre-built move-analyzer binary is not available for my platform?

If you are on Windows, the following answer assumes that your Windows user name is `USER`.

The `move-analyzer` language server is a Rust program which you can install manually provided
that you have Rust development already [installed](https://www.rust-lang.org/tools/install).
This can be done in two steps:

1. Install the move-analyzer installation prerequisites for your platform. They are the same
   as prerequisites for IOTA installation - for Linux, macOS and Windows these prerequisites and
   their installation instructions can be found
   [here](https://docs.iota.org/developer/getting-started/install-iota#additional-prerequisites-by-operating-system)
2. Invoke `cargo install --git https://github.com/iotaledger/iota iota-move-lsp` to install the
   `move-analyzer` language server in your Cargo binary directory, which is typically located
   in the `~/.cargo/bin` (macOS/Linux) or `C:\Users\USER\.cargo\bin` (Windows) directory.
3. Copy the move-analyzer binary to `~/.iota/bin` (macOS/Linux) or `C:\Users\USER\.iota\bin`
   (Windows), which is its default location (create this directory if it does not exist).

## What if I want to use a move-analyzer binary in a different location?

If you are on Windows, the following answer assumes that your Windows user name is `USER`.

If your `move-analyzer` binary is in a different directory than the default one (`~/.iota/bin`
on macOS or Linux, or `C:\Users\USER\.iota\bin` on Windows), you may have the extension look
for the binary at this new location using VSCode's settings (`⌘` + `,` on macOS, or use the menu
item _Code > Preferences > Settings_). Search for the `move.server.path` user setting,
set it to the new location of the `move-analyzer` binary, and restart VSCode.

## What if advanced features (e.g., go to def) do not work, particularly after re-install or upgrade

Assuming you did not specify a different location for the move-analyzer binary and that the
move-analyzer binary already exists in the default location (`~/.iota/bin` on macOS or Linux, or
`C:\Users\USER\.iota\bin` on Windows), delete the existing move-analyzer binary and reinstall the
extension.

## What if everything else fails?

Check the [IOTA Discord](https://discord.iota.org/) to see if the problem
has already been reported and, if not, report it there.

# Features

Here are some of the features of the Move Visual Studio Code extension. To see them, open a
Move source file (a file with a `.move` file extension) and:

- See Move keywords and types highlighted in appropriate colors.
- Comment and un-comment lines of code (`⌘` + `/` on macOS or the menu item _Edit >
  Toggle Line Comment_).
- Place your cursor on a delimiter, such as `<`, `(`, or `{`, and its corresponding delimiter --
  `>`, `)`, or `}` -- will be highlighted.
- As you type, the editor will offer completion suggestions, in particular:
  - struct field name and method name suggestions following `.` being typed
  - suggestions following `::` being typed
  - code snippets to complete `init` function and object type definitions
- If the opened Move source file is located within a buildable project (a `Move.toml` file can be
  found in one of its parent directories), the following advanced features will also be available:
  - compiler diagnostics
  - go to definition
  - go to type definition
  - go to references
  - type on hover
  - outline view showing symbol tree for Move source files
  - inlay hints:
    - types: local declarations, lambda parameters, variant and struct pattern matching
    - parameter names at function calls
- If the opened Move source file is located within a buildable project, and you have the `iota`
  binary installed, you can build and (locally)
  test this project using `Move: Build a Move package` and `Move: Test a Move package` commands from
  VSCode's command palette.
- If the opened Move source file is located within a buildable project, and you have the `iota`
  binary installed, you can trace-debug execution of Move unit tests within this project.
  This functionality is provided by this (Move) extension automatically including the Move Trace Debugging
  [extension](https://marketplace.visualstudio.com/items?itemName=mysten.move-trace-debug). Go to
  the Move Trace Debugging extension link to find more detailed information about trace-debugging and
  the current level of support. Trace-debugging a Move unit test is a two-step process:
  - first, you need to generate traces for Move unit tests by using `Move: Trace Move test execution`
    command from VSCode's command palette (traces will be available in the `traces` directory in JSON format)
  - second, you need to execute `Run->Start Debugging` menu command with Move file containing the test
    you want to trace-debug opened (if the file contains multiple tests, you will be able to select a specific one
    from a drop-down menu)
