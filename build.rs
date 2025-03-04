fn main() {
    cynic_codegen::register_schema("opencti")
        .from_sdl_file("api/opencti/opencti.graphql")
        .unwrap()
        .as_default()
        .unwrap();
}