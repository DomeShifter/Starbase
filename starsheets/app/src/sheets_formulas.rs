//! Checks that Google Sheets formulas work as Sheets users expect (patch 0022).

#[cfg(test)]
mod tests {
    use gridcraft_engine::Session;
    use gridcraft_engine::core::{CellRef, Value};
    use serde_json::json;

    fn sheet() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        for (cell, v) in [("A1", "1"), ("A2", "2"), ("B1", "10"), ("B2", "20")] {
            s.execute("cell.set", json!({"cell": cell, "input": v})).unwrap();
        }
        s
    }

    fn at(s: &Session, a1: &str) -> Value {
        s.active().unwrap().wb.active().unwrap().value(CellRef::parse(a1).unwrap())
    }

    fn input(s: &Session, a1: &str) -> String {
        s.active().unwrap().wb.active().unwrap().input_text(CellRef::parse(a1).unwrap())
    }

    #[test]
    fn braces_put_ranges_side_by_side_and_under_each_other() {
        let mut s = sheet();
        s.execute("cell.set", json!({"cell": "D1", "input": "={A1:A2, B1:B2}"})).unwrap();
        assert_eq!((at(&s, "D1"), at(&s, "E2")), (Value::number(1.0), Value::number(20.0)));
        assert_eq!(input(&s, "D1"), "={A1:A2,B1:B2}");
        s.execute("cell.set", json!({"cell": "G1", "input": "={A1:A2; B1:B2}"})).unwrap();
        assert_eq!((at(&s, "G1"), at(&s, "G4")), (Value::number(1.0), Value::number(20.0)));
        s.execute("cell.set", json!({"cell": "H1", "input": "={\"Total\", SUM(A1:B2)}"})).unwrap();
        assert_eq!(at(&s, "I1"), Value::number(33.0));
        // Constants still make an ordinary array constant.
        s.execute("cell.set", json!({"cell": "J1", "input": "={1,2;3,4}"})).unwrap();
        assert_eq!(at(&s, "K2"), Value::number(4.0));
    }

    #[test]
    fn arrayformula_spills_its_result() {
        let mut s = sheet();
        s.execute("cell.set", json!({"cell": "D1", "input": "=ARRAYFORMULA(A1:A2*B1:B2)"})).unwrap();
        assert_eq!((at(&s, "D1"), at(&s, "D2")), (Value::number(10.0), Value::number(40.0)));
        s.execute("cell.set", json!({"cell": "E1", "input": "=ARRAYFORMULA(IF(A1:A2=1,\"one\",\"other\"))"})).unwrap();
        assert_eq!(at(&s, "E2"), Value::Text("other".into()));
    }

    #[test]
    fn query_uses_the_sheet_s_column_letters() {
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        for (cell, v) in [("B1", "Name"), ("C1", "Score"), ("B2", "Ann"), ("C2", "5"), ("B3", "Bob"), ("C3", "9"), ("B4", "Cat"), ("C4", "7")] {
            s.execute("cell.set", json!({"cell": cell, "input": v})).unwrap();
        }
        s.execute("cell.set", json!({"cell": "E1", "input": "=QUERY(B1:C4, \"select B, C where C > 6 order by C desc\")"})).unwrap();
        assert_eq!(
            (at(&s, "E1"), at(&s, "E2"), at(&s, "F2"), at(&s, "E3")),
            (Value::Text("Name".into()), Value::Text("Bob".into()), Value::number(9.0), Value::Text("Cat".into()))
        );
        s.execute("cell.set", json!({"cell": "H1", "input": "=QUERY({B2:C4}, \"select Col1 where Col2 < 6\", 0)"})).unwrap();
        assert_eq!(at(&s, "H1"), Value::Text("Ann".into()));
        s.execute("cell.set", json!({"cell": "J1", "input": "=QUERY(B1:C4, \"select nonsense\")"})).unwrap();
        assert!(matches!(at(&s, "J1"), Value::Error(_)));
    }
}
