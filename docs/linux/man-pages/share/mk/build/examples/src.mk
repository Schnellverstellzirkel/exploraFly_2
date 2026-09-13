# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_BUILD_EX_SRC_INCLUDED
MAKEFILE_BUILD_EX_SRC_INCLUDED := 1


include $(MAKEFILEDIR)/build/_.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/head.mk
include $(MAKEFILEDIR)/configure/build-depends/findutils/find.mk
include $(MAKEFILEDIR)/configure/build-depends/findutils/xargs.mk
include $(MAKEFILEDIR)/configure/build-depends/grep/grep.mk
include $(MAKEFILEDIR)/configure/build-depends/mandoc/mandoc.mk
include $(MAKEFILEDIR)/configure/build-depends/sed/sed.mk
include $(MAKEFILEDIR)/configure/directory_variables/src.mk
include $(MAKEFILEDIR)/src/sortman.mk


_EX_TU_src := \
	$(patsubst $(MANDIR)/%, $(_MANDIR)/%, \
		$(shell \
			$(FIND) $(MANDIR)/* -type f \
			| $(GREP) -E '$(MANEXT)' \
			| $(XARGS) $(GREP) -H '^\.\\" SRC BEGIN ' \
			| $(SED) 's,:\.\\" SRC BEGIN (,.d/,' \
			| $(SED) 's/)//' \
			| $(SORTMAN) \
			| $(SED) 's,:,\\:,g' \
		) \
	)
_EX_TU_h := $(filter %.h, $(_EX_TU_src))
_EX_TU_c := $(filter %.c, $(_EX_TU_src))


$(_EX_TU_src): $$(patsubst %.d, %, $$(@D)) $(MK) | $$(@D)/
$(_EX_TU_c):   $$(filter $$(@D)/%.h, $(_EX_TU_h))
$(_EX_TU_src):
	$(info	$(INFO_)SED		$@)
	<$< \
	$(SED) -n \
		-e '/^\.TH/,/^\.SH/{/^\.SH/!p}' \
		-e '/^\.SH EXAMPLES/p' \
		-e "/^\... SRC BEGIN ($(@F))$$/,/^\... SRC END$$/p" \
	| $(MANDOC) -Tutf8 \
	| $(HEAD) -n-2 \
	| $(SED) '/^[^ ]/d' \
	| $(SED) 's/^       //' \
	>$@


.PHONY: build-ex-src
build-ex-src: $(_EX_TU_src);


endif  # include guard
