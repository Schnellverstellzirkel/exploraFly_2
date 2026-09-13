# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_BUILD_MAN_MAN_INCLUDED
MAKEFILE_BUILD_MAN_MAN_INCLUDED := 1


include $(MAKEFILEDIR)/build/_.mk
include $(MAKEFILEDIR)/configure/build-depends/git/git.mk
include $(MAKEFILEDIR)/configure/build-depends/sed/sed.mk
include $(MAKEFILEDIR)/configure/directory_variables/src.mk
include $(MAKEFILEDIR)/configure/verbose.mk
include $(MAKEFILEDIR)/configure/version.mk
include $(MAKEFILEDIR)/src/man.mk


_NONSO_MAN := $(patsubst $(MANDIR)/%, $(_MANDIR)/%, $(NONSO_MAN))


$(_NONSO_MAN): $(_MANDIR)/%: $(MANDIR)/% $(MK) | $$(@D)/
	$(info	$(INFO_)SED		$@)
	<$< \
	$(SED) "/^\.TH/s/(date)/$$($(MANPAGEDATECMD))/" \
	| $(SED) '/^\.TH/s/(unreleased)/$(DISTVERSION)/' >$@


.PHONY: build-man-man
build-man-man: $(_NONSO_MAN)


endif  # include guard
