#!/usr/bin/bash
# Copyright (c) 2026 Matthias Knöfel
#
# This program and the accompanying materials are made available under
# the terms of the Eclipse Public License 2.0 which accompanies this
# distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
#
# AI Disclosure: This file was mostly AI-generated.
#
# SPDX-License-Identifier: EPL-2.0 and CC0-1.0
# Assisted-by: Claude Sonnet 5.5, Claude Opus 5.5

FOUND=NO

for i in `pidof dfm_sovd_bridge` ; do
    FOUND=YES
    kill $i
    sleep 1
    kill -9 $i > /dev/null 2>&1
done

if [[ $FOUND == YES ]] ; then
    echo "DFM SOVD bridge killed"
else
    echo "No DFM SOVD bridge process found"
fi
