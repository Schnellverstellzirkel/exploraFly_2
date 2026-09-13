# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_LINT_MAN_INCLUDED
MAKEFILE_LINT_MAN_INCLUDED := 1


.PHONY: lint-man
lint-man: lint-man-mandoc lint-man-tbl lint-man-so;


endif  # include guard
