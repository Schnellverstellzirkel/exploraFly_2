# Copyright 2022-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_COREUTILS_INSTALL_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_COREUTILS_INSTALL_INCLUDED := 1


ifndef INSTALL
INSTALL := install
endif


ifndef INSTALL_DATA
INSTALL_DATA    := $(INSTALL) -m 644
endif
ifndef INSTALL_DIR
INSTALL_DIR     := $(INSTALL) -m 755 -d
endif
ifndef INSTALL_PROGRAM
INSTALL_PROGRAM := $(INSTALL) -m 755
endif


endif  # include guard
