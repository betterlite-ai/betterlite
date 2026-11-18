use super::*;

#[test]
fn cloudzero_schedule() {
    let later = moment()
        .checked_add_signed(TimeDelta::try_milliseconds(500).expect("a duration"))
        .expect("a later instant");
    let record = CbfRecord::from([("moment".to_owned(), CbfValue::DateTime(later))]);
    let preview = preview_record(&record).expect("a preview");
    assert_eq!(
        preview["moment"],
        JsonData::String("2030-01-02T03:04:05.500000".to_owned())
    );
}
