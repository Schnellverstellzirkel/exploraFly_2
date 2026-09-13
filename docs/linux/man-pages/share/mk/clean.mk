# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CLEAN_INCLUDED
MAKEFILE_CLEAN_INCLUDED := 1


include $(MAKEFILEDIR)/configure/build-depends/coreutils/rm.mk
include $(MAKEFILEDIR)/configure/directory_variables/build.mk


.PHONY: clean
clean:
	$(info	$(INFO_)RM -rf		$(builddir))
	$(RM) -rf $(builddir)


endif  # include guard
