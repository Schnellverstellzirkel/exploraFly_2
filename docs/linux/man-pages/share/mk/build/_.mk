# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_BUILD_INCLUDED
MAKEFILE_BUILD_INCLUDED := 1


include $(MAKEFILEDIR)/configure/directory_variables/build.mk


_MANDIR := $(builddir)/man


.PHONY: build-all
build-all: \
	build-catman \
	build-html \
	build-man \
	build-pdf \
	build-ps \
	build-ex;

.PHONY: build
build: \
	build-man;


endif  # include guard
