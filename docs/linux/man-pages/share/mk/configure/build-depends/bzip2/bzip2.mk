# Copyright 2023-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_BZIP2_BZIP2_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_BZIP2_BZIP2_INCLUDED := 1


DEFAULT_BZIP2FLAGS :=
ifndef BZIP2FLAGS
BZIP2FLAGS         :=
endif
ifndef BZIP2FLAGS_
BZIP2FLAGS_        := $(DEFAULT_BZIP2FLAGS) $(BZIP2FLAGS)
endif
ifndef BZIP2
BZIP2              := bzip2
endif


endif  # include guard
