# Copyright 2022-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_SHELLCHECK_SHELLCHECK_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_SHELLCHECK_SHELLCHECK_INCLUDED := 1


include $(MAKEFILEDIR)/configure/directory_variables/src.mk


SHELLCHECK_CONF         := $(SYSCONFDIR)/shellcheck/shellcheckrc
DEFAULT_SHELLCHECKFLAGS := -o all
ifndef SHELLCHECKFLAGS
SHELLCHECKFLAGS         :=
endif
ifndef SHELLCHECKFLAGS_
SHELLCHECKFLAGS_        := $(DEFAULT_SHELLCHECKFLAGS) $(SHELLCHECKFLAGS)
endif
ifndef SHELLCHECK
SHELLCHECK              := shellcheck
endif


endif  # include guard
