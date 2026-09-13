# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_INSTALL_PDF_INCLUDED
MAKEFILE_INSTALL_PDF_INCLUDED := 1


.PHONY: install-pdf
install-pdf: install-pdf-book;

.PHONY: uninstall-pdf
uninstall-pdf: uninstall-pdf-book;


endif  # include guard
