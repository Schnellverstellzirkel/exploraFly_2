# Copyright 2022-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_BINUTILS_LD_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_BINUTILS_LD_INCLUDED := 1


include $(MAKEFILEDIR)/configure/build-depends/cpp/cpp.mk
include $(MAKEFILEDIR)/configure/build-depends/gcc/cc.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/echo.mk
include $(MAKEFILEDIR)/configure/build-depends/pkgconf/pkgconf.mk
include $(MAKEFILEDIR)/configure/verbose.mk


ifndef LD
LD := $(CC) $(CFLAGS_) $(CPPFLAGS_)
endif


LD_HAS_FUSE_LINKER_PLUGIN := \
	$(shell \
		$(ECHO) 'int main(void) {}' \
		| $(LD) -fuse-linker-plugin -x c -o /dev/null /dev/stdin $(HIDE_ERR) \
		&& $(ECHO) yes \
		|| $(ECHO) no; \
	)


DEFAULT_LDFLAGS := \
	-Wl,--as-needed \
	-Wl,--no-allow-shlib-undefined \
	-Wl,--no-copy-dt-needed-entries \
	-Wl,--no-undefined \
	$(shell $(PKGCONF_CMD) --libs-only-L $(PKGCONF_LIBS) $(HIDE_ERR)) \
	$(shell $(PKGCONF_CMD) --libs-only-other $(PKGCONF_LIBS) $(HIDE_ERR))

ifeq ($(LD_HAS_FUSE_LINKER_PLUGIN),yes)
DEFAULT_LDFLAGS += -fuse-linker-plugin
endif

ifndef LDFLAGS
LDFLAGS         :=
endif
ifndef LDFLAGS_
LDFLAGS_        := $(DEFAULT_LDFLAGS) $(LDFLAGS)
endif


DEFAULT_LDLIBS := \
	-lc \
	$(shell $(PKGCONF_CMD) --libs-only-l $(PKGCONF_LIBS) $(HIDE_ERR))
ifndef LDLIBS
LDLIBS         :=
endif
ifndef LDLIBS_
LDLIBS_        := $(DEFAULT_LDLIBS) $(LDLIBS)
endif


endif  # include guard
