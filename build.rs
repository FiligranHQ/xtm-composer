fn main() {
    cynic_codegen::register_schema("opencti")
        .from_sdl_file("schemas/opencti.graphql")
        .unwrap()
        .as_default()
        .unwrap();
}