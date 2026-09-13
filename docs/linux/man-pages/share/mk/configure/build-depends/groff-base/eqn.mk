# Copyright 2024-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_BUILD_DEPENDS_GROFF_BASE_EQN_INCLUDED
MAKEFILE_CONFIGURE_BUILD_DEPENDS_GROFF_BASE_EQN_INCLUDED := 1


DEFAULT_EQNFLAGS :=
ifndef EQNFLAGS
EQNFLAGS         :=
endif
ifndef EQNFLAGS_
EQNFLAGS_        := $(DEFAULT_EQNFLAGS) $(EQNFLAGS)
endif
ifndef EQN
EQN              := eqn
endif


endif  # include guard
