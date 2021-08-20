#!/bin/bash

# cd ..

/usr/bin/find ./contracts -type d -maxdepth 1 -exec mkdir -p -- ./dist/{} \;
/usr/bin/find ./packages -type d -maxdepth 1 -exec mkdir -p -- ./dist/{} \;

for dir in contracts/*
do
  echo ${dir}
  npm run json2ts -- -i "${dir}/schema/" -o "dist/${dir}"
done

for dir in packages/*
do
  echo ${dir}
  npm run json2ts -- -i "${dir}/schema/" -o "dist/${dir}"
done

npm run generate-barrels
