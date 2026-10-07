#!/usr/bin/bash

FOUND=NO

for i in `pidof vss_publisher` ; do
    if [[ $FOUND == YES ]] ; then
        echo "More than one VSS bridge process found, gonna kill them all."
    fi
    FOUND=YES
    kill $i
    sleep 1
    kill -9 $i > /dev/null 2>&1 # eat flaming death!
done

if [[ $FOUND == YES ]] ; then
    echo "VSS bridge killed"
else
    echo "No VSS bridge process found"
fi

