
## Running Tests

To run the test suite locally, you must first compile the GSettings schemas. A helper script is provided:

```sh
./test.sh
```

Or manually:

```sh
glib-compile-schemas data/
GSETTINGS_SCHEMA_DIR="$PWD/data" GIO_USE_VFS=local cargo test
```
