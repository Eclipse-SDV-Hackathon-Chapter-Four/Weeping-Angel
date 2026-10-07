#!/usr/bin/bash

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

