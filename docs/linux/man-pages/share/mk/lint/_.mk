# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_LINT_INCLUDED
MAKEFILE_LINT_INCLUDED := 1


.PHONY: lint
lint: lint-c lint-man lint-sh;


endif  # include guard
