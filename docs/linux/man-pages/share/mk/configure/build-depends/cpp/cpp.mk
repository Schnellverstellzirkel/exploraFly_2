# Copyright 2022-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_CPP_CPP_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_CPP_CPP_INCLUDED := 1


include $(MAKEFILEDIR)/configure/build-depends/coreutils/echo.mk
include $(MAKEFILEDIR)/configure/build-depends/gcc/cc.mk
include $(MAKEFILEDIR)/configure/build-depends/grep/grep.mk
include $(MAKEFILEDIR)/configure/build-depends/pkgconf/pkgconf.mk
include $(MAKEFILEDIR)/configure/verbose.mk


ifndef CPP
CPP := $(CC) $(CFLAGS_) -E
endif


CPP_HAS_ALREADY_D_FORTIFY_SOURCE := \
	$(shell \
		$(CPP) -dM - -Wno-error </dev/null \
		| $(GREP) '\#define _FORTIFY_SOURCE ' >/dev/null \
		&& $(ECHO) yes \
		|| $(ECHO) no; \
	)


DEFAULT_CPPFLAGS := \
	$(shell $(PKGCONF_CMD) --cflags $(PKGCONF_LIBS) $(HIDE_ERR))
ifeq ($(CPP_HAS_ALREADY_D_FORTIFY_SOURCE),no)
DEFAULT_CPPFLAGS += -D_FORTIFY_SOURCE=2
endif
ifndef CPPFLAGS
CPPFLAGS         :=
endif
ifndef CPPFLAGS_
CPPFLAGS_        := $(DEFAULT_CPPFLAGS) $(CPPFLAGS)
endif


endif  # include guard
