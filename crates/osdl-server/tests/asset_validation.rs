//! Repository fixtures exercise the public full-validation pipeline. Production
//! callers are not tied to this location and may validate any external package.

use std::path::{Path, PathBuf};

#[test]
fn every_instrument_package_passes_static_and_dynamic_validation() {
    let root = instrument_root();
    let mut packages = 0;
    for entry in std::fs::read_dir(root).unwrap() {
        let package = entry.unwrap().path().join("package");
        if !package.join("asset.yaml").is_file() {
            continue;
        }
        let report = osdl_server::simulation_validation::validate_directory(&package)
            .unwrap_or_else(|error| panic!("{}: {error:#}", package.display()));
        let dynamic = report.dynamic.expect("instrument has dynamic evidence");
        assert!(dynamic.actions_executed >= 4, "{}", report.name);
        assert!(
            dynamic.observations_checked > dynamic.actions_executed,
            "{}",
            report.name
        );
        assert_eq!(dynamic.engine, "kinematic");
        packages += 1;
    }
    assert_eq!(packages, 22);
}

#[test]
fn a_corrupt_glb_container_is_rejected_before_dynamic_execution() {
    let temp = tempfile::tempdir().unwrap();
    let source = instrument_root().join("analytical-balance/package");
    let destination = temp.path().join("external-analytical-balance");
    copy_tree(&source, &destination);

    // Extend the file without updating its container length. This is a minimal
    // corruption fixture; dedicated GLB unit tests cover accessor arithmetic.
    let preview = destination.join("preview.glb");
    let mut bytes = std::fs::read(&preview).unwrap();
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    std::fs::write(preview, bytes).unwrap();

    let error = osdl_server::simulation_validation::validate_directory(&destination).unwrap_err();
    assert!(format!("{error:#}").contains("GLB"), "{error:#}");
}

#[test]
fn dynamic_calls_must_obey_the_public_action_schema() {
    let temp = tempfile::tempdir().unwrap();
    let source = instrument_root().join("thermal-cycler/package");
    let destination = temp.path().join("external-thermal-cycler");
    copy_tree(&source, &destination);

    // set_cycles has no GLB control whose generated parameters could catch
    // this mutation during initial admission. The controller accepts numeric
    // cycle counts, while this deliberately conflicting public schema does not.
    let world = destination.join("source/world.yaml");
    let yaml = std::fs::read_to_string(&world)
        .unwrap()
        .replace(
            "\"cycles\": {\n                \"type\": \"integer\",\n                \"minimum\": 1,\n                \"maximum\": 10\n              }",
            "\"cycles\": {\n                \"type\": \"string\"\n              }",
        );
    assert_ne!(yaml, std::fs::read_to_string(&world).unwrap());
    std::fs::write(world, yaml).unwrap();

    let error = osdl_server::simulation_validation::validate_directory(&destination).unwrap_err();
    let error = format!("{error:#}");
    assert!(error.contains("dynamic behavior stage failed"), "{error}");
    assert!(error.contains("set_cycles"), "{error}");
    assert!(error.contains("violates its declared schema"), "{error}");
}

#[test]
fn schema_declarations_must_be_typed_and_unambiguous() {
    for duplicate in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let source = instrument_root().join("thermal-cycler/package");
        let destination = temp.path().join("thermal-cycler");
        copy_tree(&source, &destination);
        let world = destination.join("source/world.yaml");
        let mut definition: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(&world).unwrap()).unwrap();
        if duplicate {
            let duplicate_action = definition["devices"][0]["actions"][7].clone();
            definition["devices"][0]["actions"]
                .as_sequence_mut()
                .unwrap()
                .push(duplicate_action);
        } else {
            definition["devices"][0]["actions"][7]["params"]["required"] =
                serde_yaml::Value::String("cycles".to_owned());
        }
        std::fs::write(&world, serde_yaml::to_string(&definition).unwrap()).unwrap();

        let error = format!(
            "{:#}",
            osdl_server::simulation_validation::validate_directory(&destination).unwrap_err()
        );
        if duplicate {
            assert!(error.contains("duplicate action"), "{error}");
        } else {
            assert!(error.contains("required must be an array"), "{error}");
        }
    }
}

#[test]
fn validated_packaging_preserves_the_exact_manifest_digest() {
    let temp = tempfile::tempdir().unwrap();
    let source = instrument_root().join("analytical-balance/package");
    let destination = temp.path().join("analytical-balance");
    copy_tree(&source, &destination);
    let expected = opensdl_assets::pack_directory(&destination, &temp.path().join("direct"))
        .unwrap()
        .manifest_digest;
    let actual = osdl_server::simulation_validation::pack_directory(
        &destination,
        &temp.path().join("validated"),
    )
    .unwrap()
    .manifest_digest;
    assert_eq!(actual, expected);
}

fn instrument_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/instruments")
}

fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}
