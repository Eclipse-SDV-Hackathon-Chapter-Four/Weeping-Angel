#!/usr/bin/bash

FOUND=NO

for i in `pidof opensovd-gateway` ; do
    if [[ $FOUND == YES ]] ; then
        echo "More than one OpenSOVD process found, gonna kill them all."
    fi
    FOUND=YES
    kill $i
    sleep 1
    kill -9 $i > /dev/null 2>&1  # eat flaming death!
done

if [[ $FOUND == YES ]] ; then
    echo "OpenSOVD killed"
else
    echo "No OpenSOVD process found"
fi

