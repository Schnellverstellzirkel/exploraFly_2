# Copyright 2022-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_CPPLINT_CPPLINT_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_CPPLINT_CPPLINT_INCLUDED := 1


include $(MAKEFILEDIR)/configure/directory_variables/src.mk


CPPLINT_CONF         := $(SYSCONFDIR)/cpplint/cpplint.cfg
DEFAULT_CPPLINTFLAGS :=
ifndef CPPLINTFLAGS
CPPLINTFLAGS         :=
endif
ifndef CPPLINTFLAGS_
CPPLINTFLAGS_        := $(DEFAULT_CPPLINTFLAGS) $(CPPLINTFLAGS)
endif
ifndef CPPLINT
CPPLINT              := cpplint
endif


endif  # include guard
