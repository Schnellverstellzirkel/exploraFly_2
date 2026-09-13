# Copyright 2024-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_GCC_CC_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_GCC_CC_INCLUDED := 1


include $(MAKEFILEDIR)/configure/build-depends/coreutils/echo.mk
include $(MAKEFILEDIR)/configure/build-depends/grep/grep.mk
include $(MAKEFILEDIR)/configure/build-depends/sed/sed.mk


ifndef CC
CC := gcc
endif


CC_VENDOR := \
	$(shell \
		$(CC) -v 2>&1 \
		| $(SED) -n '1p;$$p' \
		| $(SED) '/gcc version/s/.*/gcc/' \
		| $(SED) '/clang version/s/.*/clang/' \
		| $(SED) '/Apple LLVM version/s/.*/clang/' \
		| $(GREP) -e '^gcc$$' -e '^clang$$' \
		|| $(ECHO) unknown; \
	)


COMMON_CFLAGS := \
	-O3 \
	-flto \
	-Wall \
	-Wextra \
	-Werror \
	-Wstrict-prototypes \
	-Wdeclaration-after-statement \
	-Wno-reserved-identifier \
	-Wno-unused-macros \
	-Wno-error=unused-parameter \
	-Wno-error=sign-compare \
	-Wno-error=format \
	-Wno-error=uninitialized


GCC_CFLAGS := -fanalyzer


CLANG_CFLAGS := \
	-Weverything \
	-Wno-language-extension-token \
	-Wno-unreachable-code \
	-Wno-unsafe-buffer-usage \
	-Wno-used-but-marked-unused


DEFAULT_CFLAGS := $(COMMON_CFLAGS)

ifeq ($(CC_VENDOR),gcc)
DEFAULT_CFLAGS += $(GCC_CFLAGS)
else ifeq ($(CC_VENDOR),clang)
DEFAULT_CFLAGS += $(CLANG_CFLAGS)
endif

ifndef CFLAGS
CFLAGS         :=
endif
ifndef CFLAGS_
CFLAGS_        := $(DEFAULT_CFLAGS) $(CFLAGS)
endif


endif  # include guard
