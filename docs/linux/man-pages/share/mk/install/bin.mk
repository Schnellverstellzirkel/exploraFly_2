# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_INSTALL_BIN_INCLUDED
MAKEFILE_INSTALL_BIN_INCLUDED := 1


include $(MAKEFILEDIR)/configure/build-depends/coreutils/install.mk
include $(MAKEFILEDIR)/configure/directory_variables/install.mk
include $(MAKEFILEDIR)/configure/directory_variables/src.mk
include $(MAKEFILEDIR)/src/sh.mk


_bindir := $(DESTDIR)$(bindir)


_bin_sh := $(patsubst $(SRCBINDIR)/%, $(_bindir)/%, $(BIN_sh))
_bin    := $(_bin_sh)
_bin_rm := $(addsuffix -rm, $(wildcard $(_bin)))


$(_bin_sh): $(_bindir)/%: $(SRCBINDIR)/%
$(_bin): $(MK) | $$(@D)/


$(_bin):
	$(info	$(INFO_)INSTALL		$@)
	$(INSTALL_PROGRAM) -T $< $@


.PHONY: install-bin
install-bin: $(_bin);

.PHONY: uninstall-bin
uninstall-bin: $(_bin_rm);


endif  # include guard
