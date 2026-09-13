# Copyright 2023-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_GZIP_GZIP_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_GZIP_GZIP_INCLUDED := 1


DEFAULT_GZIPFLAGS := -n
ifndef GZIPFLAGS
GZIPFLAGS         :=
endif
ifndef GZIPFLAGS_
GZIPFLAGS_        := $(DEFAULT_GZIPFLAGS) $(GZIPFLAGS)
endif
ifndef GZIP
GZIP              := gzip
endif


endif  # include guard
