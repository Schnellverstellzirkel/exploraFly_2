# Copyright 2021-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_DIRECTORY_VARIABLES_INSTALL_INCLUDED
MAKEFILE_CONFIGURE_DIRECTORY_VARIABLES_INSTALL_INCLUDED := 1


include $(MAKEFILEDIR)/configure/directory_variables/src.mk


ifndef DESTDIR
DESTDIR     :=
endif
ifndef prefix
prefix      := /usr/local
endif
ifndef exec_prefix
exec_prefix := $(prefix)
endif
ifndef datarootdir
datarootdir := $(prefix)/share
endif
ifndef mandir
mandir      := $(datarootdir)/man
endif
ifndef docdir
docdir      := $(datarootdir)/doc
endif
ifndef htmldir
htmldir     := $(docdir)/html/man
endif
ifndef pdfdir
pdfdir      := $(docdir)/pdf
endif
ifndef bindir
bindir      := $(exec_prefix)/bin
endif


$(foreach s, $(MANSECTIONS),                                                  \
	$(eval man$(s)dir ?= $(mandir)/man$(s)))
$(foreach s, $(MANSECTIONS),                                                  \
	$(eval man$(s)ext ?= .$(s)))


endif  # include guard
