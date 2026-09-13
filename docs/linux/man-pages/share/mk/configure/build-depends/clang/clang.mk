# Copyright 2024-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_CLANG_CLANG_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_CLANG_CLANG_INCLUDED := 1


include $(MAKEFILEDIR)/configure/build-depends/gcc/cc.mk


DEFAULT_CLANGFLAGS := \
	$(COMMON_CFLAGS) \
	$(CLANG_CFLAGS)
ifndef CLANGFLAGS
CLANGFLAGS         :=
endif
ifndef CLANGFLAGS_
CLANGFLAGS_        := $(DEFAULT_CLANGFLAGS) $(CLANGFLAGS)
endif


endif  # include guard
