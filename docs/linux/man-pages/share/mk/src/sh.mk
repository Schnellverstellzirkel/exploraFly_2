# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_SRC_SH_INCLUDED
MAKEFILE_SRC_SH_INCLUDED := 1


include $(MAKEFILEDIR)/configure/build-depends/coreutils/sort.mk
include $(MAKEFILEDIR)/configure/build-depends/findutils/find.mk
include $(MAKEFILEDIR)/configure/build-depends/findutils/xargs.mk
include $(MAKEFILEDIR)/configure/build-depends/grep/grep.mk
include $(MAKEFILEDIR)/configure/directory_variables/src.mk


BIN_sh := $(shell $(FIND) $(SRCBINDIR) -type f \
		| $(XARGS) $(GREP) -l '^\#!/bin/\(sh\|bash\)\>' \
		| $(SORT))


endif  # include guard
