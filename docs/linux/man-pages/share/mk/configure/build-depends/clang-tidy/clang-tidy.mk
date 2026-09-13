# Copyright 2022-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_CLANG_TIDY_CLANG_TIDY_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_CLANG_TIDY_CLANG_TIDY_INCLUDED := 1


include $(MAKEFILEDIR)/configure/directory_variables/src.mk


CLANG_TIDY_CONF         := $(SYSCONFDIR)/clang-tidy/config.yaml
DEFAULT_CLANG_TIDYFLAGS := \
	--config-file=$(CLANG_TIDY_CONF) \
	--quiet \
	--use-color
ifndef CLANG_TIDYFLAGS
CLANG_TIDYFLAGS         :=
endif
ifndef CLANG_TIDYFLAGS_
CLANG_TIDYFLAGS_        := $(DEFAULT_CLANG_TIDYFLAGS) $(CLANG_TIDYFLAGS)
endif
ifndef CLANG_TIDY
CLANG_TIDY              := clang-tidy
endif


endif  # include guard
