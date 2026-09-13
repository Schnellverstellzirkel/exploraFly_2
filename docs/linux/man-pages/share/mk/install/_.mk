# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_INSTALL_INCLUDED
MAKEFILE_INSTALL_INCLUDED := 1


include $(MAKEFILEDIR)/configure/build-depends/coreutils/install.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/rm.mk


%/:
	+$(info	$(INFO_)MKDIR		$@)
	+$(INSTALL_DIR) $@

%-rm:
	$(info	$(INFO_)RM		$*)
	$(RM) $*


.PHONY: install-all
install-all: install-man install-bin;

.PHONY: install
install: install-man install-bin;

.PHONY: uninstall
uninstall: uninstall-man;


endif  # include guard
