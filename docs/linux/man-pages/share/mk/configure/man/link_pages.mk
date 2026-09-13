# Copyright 2023-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_MAN_LINK_PAGES_INCLUDED
MAKEFILE_CONFIGURE_MAN_LINK_PAGES_INCLUDED := 1


ifndef LINK_PAGES
LINK_PAGES := .so
endif
ifeq ($(LINK_PAGES),.so)
else ifeq ($(LINK_PAGES),symlink)
else
$(warning "LINK_PAGES": "$(LINK_PAGES)")
$(error Valid values for "LINK_PAGES": [".so", "symlink"])
endif


endif  # include guard
