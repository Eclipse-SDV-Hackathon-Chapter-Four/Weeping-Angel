#!/usr/bin/bash
# Copyright (c) 2026 Michael Warmuth-Uhl
#
# This program and the accompanying materials are made available under
# the terms of the Eclipse Public License 2.0 which accompanies this
# distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
#
# AI Disclosure: This file was mostly AI-generated.
#
# SPDX-License-Identifier: EPL-2.0 and CC0-1.0

FOUND=NO

for i in `pidof dfm_bin` ; do
    if [[ $FOUND == YES ]] ; then
        echo "More than one DFM process found, gonna kill them all."
    fi
    FOUND=YES
    kill $i
    sleep 1
    kill -9 $i > /dev/null 2>&1 # eat flaming death!
done

if [[ $FOUND == YES ]] ; then
    echo "DFM killed"
else
    echo "No DFM process found"
fi

