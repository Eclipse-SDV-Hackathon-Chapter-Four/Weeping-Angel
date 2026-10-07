#!/usr/bin/bash

FOUND=NO

for i in `pidof guardian` ; do
    FOUND=YES
    kill $i
    sleep 1
    kill -9 $i > /dev/null 2>&1
done

if [[ $FOUND == YES ]] ; then
    echo "Guardian killed"
else
    echo "No Guardian process found"
fi
